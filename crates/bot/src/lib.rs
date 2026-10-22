//! Bots for solo practice and automated tests.
//!
//! A bot only sees the game the way a player would: the opponent's position as
//! it was `reaction_ms` ago (plus a partial velocity lead), and it turns its
//! view with a limited speed and some aim noise. Movement follows the strafe
//! patterns from the strafe-aim / dodge guides, defined relative to the line
//! between the two players. Everything is driven by a seeded RNG, so a bot with
//! the same seed and the same game produces the same inputs.

use lgduel_sim::game::{Game, PlayerId};
use lgduel_sim::math::{Angle, Vec3, look_angles};
use lgduel_sim::tick::Tick;
use lgduel_sim::usercmd::{MOVE_MAX, UserCmd, buttons};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrafePattern {
    /// Stand still (target practice).
    Still,
    /// Random A/D: switch direction after a random hold of min..max ms.
    RandomAd { min_ms: u32, max_ms: u32 },
    /// Move the same lateral direction as the opponent (relative speed ≈ 0).
    Mirror,
    /// Move opposite to the opponent (relative speed ≈ 2v).
    AntiMirror,
    /// Half-sideways (diagonal) in the opponent's lateral direction (≈ 0.29v).
    HswMirror,
    /// Half-sideways opposite to the opponent (≈ 1.71v).
    HswAntiMirror,
}

impl StrafePattern {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "still" => Self::Still,
            "ad" | "random" => Self::RandomAd {
                min_ms: 150,
                max_ms: 600,
            },
            "mirror" => Self::Mirror,
            "anti" | "antimirror" => Self::AntiMirror,
            "hsw" => Self::HswMirror,
            "hswanti" => Self::HswAntiMirror,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AimSkill {
    /// Perception delay of the opponent's position.
    pub reaction_ms: u32,
    /// Fraction (0..1) of the opponent's velocity the bot leads by over its reaction time.
    pub lead: f32,
    /// Max view turn speed, degrees per second.
    pub turn_speed: f32,
    /// Fraction of the remaining aim error corrected each tick (0..1).
    pub gain: f32,
    /// Random aim jitter amplitude, degrees.
    pub noise_deg: f32,
}

impl AimSkill {
    pub const AIMBOT: AimSkill = AimSkill {
        reaction_ms: 0,
        lead: 0.0,
        turn_speed: 100_000.0,
        gain: 1.0,
        noise_deg: 0.0,
    };
    pub const HARD: AimSkill = AimSkill {
        reaction_ms: 120,
        lead: 0.7,
        turn_speed: 900.0,
        gain: 0.45,
        noise_deg: 0.6,
    };
    pub const MEDIUM: AimSkill = AimSkill {
        reaction_ms: 180,
        lead: 0.4,
        turn_speed: 600.0,
        gain: 0.3,
        noise_deg: 1.2,
    };
    pub const EASY: AimSkill = AimSkill {
        reaction_ms: 250,
        lead: 0.0,
        turn_speed: 360.0,
        gain: 0.18,
        noise_deg: 2.5,
    };

    pub fn preset(name: &str) -> Option<AimSkill> {
        Some(match name {
            "aimbot" => Self::AIMBOT,
            "hard" => Self::HARD,
            "medium" => Self::MEDIUM,
            "easy" => Self::EASY,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotConfig {
    pub pattern: StrafePattern,
    pub aim: AimSkill,
    /// Hold fire whenever the opponent is in range.
    pub fire: bool,
    /// Chance (0..1) to jump when changing strafe direction.
    pub jump_chance: f32,
    /// Preferred distance band to the opponent.
    pub min_range: f32,
    pub max_range: f32,
    pub seed: u64,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            pattern: StrafePattern::RandomAd {
                min_ms: 150,
                max_ms: 600,
            },
            aim: AimSkill::MEDIUM,
            fire: true,
            jump_chance: 0.1,
            min_range: 200.0,
            max_range: 450.0,
            seed: 1,
        }
    }
}

/// Small deterministic PRNG (PCG-style LCG output).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5DEE_CE66_D1CE_4E5B)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 23) as f32
    }
    /// Uniform in [-1, 1).
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

pub struct Bot {
    pub cfg: BotConfig,
    rng: Rng,
    /// Opponent (tick, origin, velocity) history for delayed perception.
    seen: VecDeque<(Tick, Vec3, Vec3)>,
    /// Current view (degrees, f64 for smooth turning).
    yaw: f64,
    pitch: f64,
    initialised: bool,
    /// Current strafe direction (+1 right / -1 left) and ticks left before re-deciding.
    strafe: i8,
    hold: u32,
    forward: i8,
    jump: bool,
}

/// Signed shortest angle difference a - b in degrees, in (-180, 180].
fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = (a - b) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

impl Bot {
    pub fn new(cfg: BotConfig) -> Self {
        Self {
            rng: Rng::new(cfg.seed),
            cfg,
            seen: VecDeque::new(),
            yaw: 0.0,
            pitch: 0.0,
            initialised: false,
            strafe: 1,
            hold: 0,
            forward: 0,
            jump: false,
        }
    }

    /// Opponent as perceived now: the state `reaction_ms` ago plus a partial lead.
    fn perceive(&self, rate: u32) -> Option<Vec3> {
        let (latest, _, _) = *self.seen.back()?;
        let delay_ticks = (self.cfg.aim.reaction_ms * rate).div_ceil(1000);
        let want = latest.0.saturating_sub(delay_ticks);
        let (_, pos, vel) = *self
            .seen
            .iter()
            .find(|(t, _, _)| t.0 >= want)
            .unwrap_or(self.seen.back()?);
        let lead_t = self.cfg.aim.reaction_ms as f32 / 1000.0 * self.cfg.aim.lead;
        Some(pos + vel * lead_t)
    }

    /// Produce this tick's command for player `me` against its first opponent.
    pub fn think(&mut self, game: &Game, me: PlayerId) -> UserCmd {
        let tick = game.tick;
        let rate = game.cfg.tick_rate_hz;
        let dt = 1.0 / rate as f64;
        let Some(p) = game.player(me) else {
            return UserCmd {
                tick,
                ..UserCmd::default()
            };
        };
        if !self.initialised {
            self.yaw = p.yaw.degrees();
            self.pitch = p.pitch.signed_degrees();
            self.initialised = true;
        }
        let Some(opp) = game.players.iter().find(|o| o.id != me && o.alive) else {
            self.seen.clear();
            return self.cmd(tick, 0, 0, false);
        };
        // Remember what the opponent did (for reaction-delayed perception).
        self.seen.push_back((tick, opp.pm.origin, opp.pm.velocity));
        let keep = (self.cfg.aim.reaction_ms * rate).div_ceil(1000) + 2;
        while self.seen.len() as u32 > keep {
            self.seen.pop_front();
        }
        if !p.alive {
            return self.cmd(tick, 0, 0, false);
        }
        let phys = &game.cfg.physics;
        let eye = p.pm.eye(phys);

        // --- aim ---
        let target = self.perceive(rate).unwrap_or(opp.pm.origin) + Vec3::new(0.0, 0.0, 8.0);
        let (tp, ty) = look_angles(target - eye);
        let (want_pitch, want_yaw) = (tp.signed_degrees(), ty.degrees());
        let a = &self.cfg.aim;
        let max_step = a.turn_speed as f64 * dt;
        let dyaw = (angle_diff(want_yaw, self.yaw) * a.gain as f64).clamp(-max_step, max_step);
        let dpitch = ((want_pitch - self.pitch) * a.gain as f64).clamp(-max_step, max_step);
        self.yaw = (self.yaw + dyaw).rem_euclid(360.0);
        self.pitch = (self.pitch + dpitch).clamp(-89.0, 89.0);
        let noise_y = self.rng.signed() as f64 * a.noise_deg as f64;
        let noise_p = self.rng.signed() as f64 * a.noise_deg as f64 * 0.5;

        // --- movement ---
        let to_opp = opp.pm.origin - p.pm.origin;
        let dist = to_opp.length_xy();
        // Lateral axis: my right when facing the opponent.
        let fwd = Vec3::new(to_opp.x, to_opp.y, 0.0).normalize();
        let lateral = Vec3::new(fwd.y, -fwd.x, 0.0);
        let range_fwd: i8 = if dist > self.cfg.max_range {
            MOVE_MAX
        } else if dist < self.cfg.min_range {
            -MOVE_MAX
        } else {
            0
        };
        // Opponent's lateral direction as perceived (delayed), in my right/left terms.
        let opp_vel = self.seen.front().map(|s| s.2).unwrap_or(Vec3::ZERO);
        let opp_lat = opp_vel.dot(lateral);
        let follow = if opp_lat.abs() > 20.0 {
            opp_lat.signum() as i8
        } else {
            self.strafe
        };

        let (right, forward) = match self.cfg.pattern {
            StrafePattern::Still => (0, 0),
            StrafePattern::RandomAd { min_ms, max_ms } => {
                if self.hold == 0 {
                    self.strafe = if self.rng.next_u32() & 1 == 0 { 1 } else { -1 };
                    let ms = min_ms + self.rng.next_u32() % (max_ms - min_ms).max(1);
                    self.hold = (ms * rate).div_ceil(1000).max(1);
                    self.jump = self.rng.unit() < self.cfg.jump_chance;
                }
                self.hold -= 1;
                (self.strafe * MOVE_MAX, range_fwd)
            }
            StrafePattern::Mirror => {
                self.strafe = follow;
                (self.strafe * MOVE_MAX, range_fwd)
            }
            StrafePattern::AntiMirror => {
                self.strafe = follow;
                (-self.strafe * MOVE_MAX, range_fwd)
            }
            StrafePattern::HswMirror | StrafePattern::HswAntiMirror => {
                self.strafe = follow;
                let s = if self.cfg.pattern == StrafePattern::HswMirror {
                    1
                } else {
                    -1
                };
                // Diagonal: always some forward/back; flip near the range limits.
                if range_fwd != 0 || self.forward == 0 {
                    self.forward = if range_fwd != 0 { range_fwd } else { MOVE_MAX };
                }
                (s * self.strafe * MOVE_MAX, self.forward)
            }
        };
        let jump = self.jump && matches!(self.cfg.pattern, StrafePattern::RandomAd { .. });
        let fire = self.cfg.fire && dist <= game.cfg.lg.range;
        let mut c = self.cmd(tick, forward, right, jump);
        c.yaw = Angle::from_degrees(self.yaw + noise_y);
        c.pitch = Angle::from_degrees(self.pitch + noise_p);
        if fire {
            c.buttons |= buttons::FIRE;
        }
        c.sanitized()
    }

    fn cmd(&self, tick: Tick, forward: i8, right: i8, jump: bool) -> UserCmd {
        UserCmd {
            tick,
            pitch: Angle::from_degrees(self.pitch),
            yaw: Angle::from_degrees(self.yaw),
            forward,
            right,
            buttons: if jump { buttons::JUMP } else { 0 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_sim::config::GameConfig;
    use lgduel_sim::world::{World, builtin};

    fn game() -> (Game, PlayerId, PlayerId) {
        let mut cfg = GameConfig::default();
        cfg.lg.knockback = 0.0;
        let p = &cfg.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let mut g = Game::new(
            cfg,
            World::from_toml_str(builtin::ARENA_FLAT, hull).unwrap(),
        );
        let a = g.add_player();
        let b = g.add_player();
        // Bring them to mid range.
        let z = g.player(a).unwrap().pm.origin.z;
        g.player_mut(a).unwrap().pm.origin = Vec3::new(-150.0, 0.0, z);
        g.player_mut(b).unwrap().pm.origin = Vec3::new(150.0, 0.0, z);
        (g, a, b)
    }

    /// Run `ticks` with both sides botted; returns per-tick (a, b) velocities.
    fn run(
        g: &mut Game,
        a: PlayerId,
        b: PlayerId,
        ba: &mut Bot,
        bb: &mut Bot,
        ticks: u32,
    ) -> Vec<(Vec3, Vec3)> {
        let mut out = vec![];
        for _ in 0..ticks {
            let ca = ba.think(g, a);
            let cb = bb.think(g, b);
            g.step(&[(a, ca), (b, cb)]);
            out.push((
                g.player(a).unwrap().pm.velocity,
                g.player(b).unwrap().pm.velocity,
            ));
        }
        out
    }

    fn cfg(pattern: StrafePattern, aim: AimSkill, fire: bool, seed: u64) -> BotConfig {
        BotConfig {
            pattern,
            aim,
            fire,
            jump_chance: 0.0,
            seed,
            ..BotConfig::default()
        }
    }

    #[test]
    fn aimbot_hits_still_target_every_cell() {
        let (mut g, a, b) = game();
        let mut shooter = Bot::new(cfg(StrafePattern::Still, AimSkill::AIMBOT, true, 1));
        let mut target = Bot::new(cfg(StrafePattern::Still, AimSkill::AIMBOT, false, 2));
        g.player_mut(b).unwrap().health = 100_000;
        run(&mut g, a, b, &mut shooter, &mut target, 250);
        let s = g.player(a).unwrap().stats;
        assert!(s.shots >= 39, "{s:?}");
        assert_eq!(
            s.hits, s.shots,
            "aimbot vs still target must hit every cell"
        );
    }

    #[test]
    fn reaction_time_lowers_accuracy_vs_strafer() {
        let mut acc = vec![];
        for aim in [AimSkill::AIMBOT, AimSkill::HARD, AimSkill::EASY] {
            let (mut g, a, b) = game();
            g.player_mut(b).unwrap().health = 1_000_000;
            g.player_mut(a).unwrap().health = 1_000_000;
            let mut shooter = Bot::new(cfg(StrafePattern::Still, aim, true, 3));
            let ad = StrafePattern::RandomAd {
                min_ms: 150,
                max_ms: 500,
            };
            let mut target = Bot::new(cfg(ad, AimSkill::AIMBOT, false, 4));
            run(&mut g, a, b, &mut shooter, &mut target, 125 * 20);
            acc.push(g.player(a).unwrap().stats.accuracy_pct());
        }
        println!("accuracy aimbot/hard/easy vs random A/D: {acc:?}");
        assert!(acc[0] > 95.0, "{acc:?}");
        assert!(acc[0] > acc[1] && acc[1] > acc[2], "{acc:?}");
        assert!(acc[2] > 5.0, "easy bot should still land some: {acc:?}");
    }

    /// Mean |relative velocity perpendicular to the line between the players| / 320,
    /// after a 1 s warm-up. Measured against the current line each tick, because
    /// anti-mirroring players orbit each other.
    fn mean_rel_lateral(
        g: &mut Game,
        a: PlayerId,
        b: PlayerId,
        ba: &mut Bot,
        bb: &mut Bot,
        ticks: u32,
    ) -> f32 {
        let mut sum = 0.0;
        let mut n = 0;
        for i in 0..ticks {
            let ca = ba.think(g, a);
            let cb = bb.think(g, b);
            g.step(&[(a, ca), (b, cb)]);
            let (pa, pb) = (g.player(a).unwrap().pm, g.player(b).unwrap().pm);
            let line =
                Vec3::new(pb.origin.x - pa.origin.x, pb.origin.y - pa.origin.y, 0.0).normalize();
            let perp = Vec3::new(line.y, -line.x, 0.0);
            if i >= 125 {
                sum += (pa.velocity - pb.velocity).dot(perp).abs() / 320.0;
                n += 1;
            }
        }
        sum / n as f32
    }

    #[test]
    fn strafe_patterns_produce_expected_relative_speeds() {
        let ad = StrafePattern::RandomAd {
            min_ms: 400,
            max_ms: 900,
        };
        let mut res = vec![];
        for pattern in [
            StrafePattern::Mirror,
            StrafePattern::AntiMirror,
            StrafePattern::HswMirror,
            StrafePattern::HswAntiMirror,
        ] {
            let (mut g, a, b) = game();
            // Zero reaction so the pattern itself is measured.
            let aim = AimSkill {
                reaction_ms: 0,
                ..AimSkill::AIMBOT
            };
            let band = |c: BotConfig| BotConfig {
                min_range: 200.0,
                max_range: 400.0,
                ..c
            };
            let mut follower = Bot::new(band(cfg(pattern, aim, false, 5)));
            let mut leader = Bot::new(band(cfg(ad, aim, false, 6)));
            res.push(mean_rel_lateral(
                &mut g,
                a,
                b,
                &mut follower,
                &mut leader,
                125 * 8,
            ));
        }
        println!(
            "mean relative lateral speed (v): mirror {:.2} anti {:.2} hsw {:.2} hswanti {:.2}",
            res[0], res[1], res[2], res[3]
        );
        // Ideal steady-state values 0 / 2 / 0.29 / 1.71; direction changes pull them toward 1.
        assert!(res[0] < 0.35, "mirror {res:?}");
        assert!(res[1] > 1.5, "anti {res:?}");
        assert!((0.1..0.6).contains(&res[2]), "hsw {res:?}");
        assert!((1.3..1.85).contains(&res[3]), "hsw anti {res:?}");
        assert!(
            res[0] < res[2] && res[2] < res[3] && res[3] < res[1],
            "ordering {res:?}"
        );
    }

    #[test]
    fn random_ad_respects_hold_bounds() {
        let (mut g, a, b) = game();
        let mut bot = Bot::new(cfg(
            StrafePattern::RandomAd {
                min_ms: 200,
                max_ms: 400,
            },
            AimSkill::MEDIUM,
            false,
            9,
        ));
        let mut idle = Bot::new(cfg(StrafePattern::Still, AimSkill::MEDIUM, false, 10));
        let mut last = 0i8;
        let mut run_len = 0u32;
        let mut runs = vec![];
        for _ in 0..125 * 20 {
            let c = bot.think(&g, a);
            let ci = idle.think(&g, b);
            g.step(&[(a, c), (b, ci)]);
            if c.right == last {
                run_len += 1;
            } else {
                if last != 0 {
                    runs.push(run_len);
                }
                last = c.right;
                run_len = 1;
            }
        }
        // Same direction may be re-picked, so runs are multiples of holds: ≥ 25 ticks (200 ms).
        assert!(runs.len() > 10);
        assert!(runs.iter().all(|r| *r >= 25), "{runs:?}");
    }

    #[test]
    fn same_seed_same_inputs() {
        let trace = |seed| {
            let (mut g, a, b) = game();
            let mut x = Bot::new(cfg(
                StrafePattern::RandomAd {
                    min_ms: 150,
                    max_ms: 600,
                },
                AimSkill::MEDIUM,
                true,
                seed,
            ));
            let mut y = Bot::new(cfg(StrafePattern::Mirror, AimSkill::HARD, true, seed + 1));
            run(&mut g, a, b, &mut x, &mut y, 1000);
            g.state_hash()
        };
        assert_eq!(trace(42), trace(42));
        assert_ne!(trace(42), trace(43));
    }

    #[test]
    fn better_aim_wins_vamp_duel() {
        let (mut g, a, b) = game();
        let ad = StrafePattern::RandomAd {
            min_ms: 150,
            max_ms: 600,
        };
        let mut good = Bot::new(cfg(ad, AimSkill::HARD, true, 11));
        let mut bad = Bot::new(cfg(ad, AimSkill::EASY, true, 12));
        run(&mut g, a, b, &mut good, &mut bad, 125 * 60);
        let (sa, sb) = (g.player(a).unwrap().stats, g.player(b).unwrap().stats);
        println!(
            "vamp 60 s: hard {:.1}% {}W  vs  easy {:.1}% {}W",
            sa.accuracy_pct(),
            sa.fights_won,
            sb.accuracy_pct(),
            sb.fights_won
        );
        assert!(sa.accuracy_pct() > sb.accuracy_pct());
        assert!(sa.fights_won > sb.fights_won);
    }

    #[test]
    fn parsing() {
        assert_eq!(StrafePattern::parse("mirror"), Some(StrafePattern::Mirror));
        assert!(StrafePattern::parse("x").is_none());
        assert_eq!(AimSkill::preset("hard"), Some(AimSkill::HARD));
        assert_eq!(angle_diff(10.0, 350.0), 20.0);
        assert_eq!(angle_diff(350.0, 10.0), -20.0);
    }
}
