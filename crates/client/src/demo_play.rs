//! Demo playback. Local demos are re-simulated tick by tick from their
//! recorded commands (and verified against the stored state hash); net demos
//! show the recorded snapshots. Both play from the recording player's view
//! with an input overlay of what they pressed.

use crate::hud::InputView;
use crate::local::LocalSession;
use crate::scene::{FrameView, OtherView, SpectateView};
use lgduel_net::demo::{Demo, DemoFrame, DemoKind};
use lgduel_net::snapshot::Snapshot;
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{Game, PlayerId};
use lgduel_sim::math::Vec3;
use lgduel_sim::usercmd::{UserCmd, buttons};
use lgduel_sim::world::{World, builtin};
use std::collections::HashMap;

/// What a finished local playback found.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The re-simulated state hash equals the recorded one.
    Match(u64),
    Mismatch {
        recorded: u64,
        replayed: u64,
    },
    /// The recording has no end marker (it was cut off).
    Unverified,
}

pub struct LocalReplay {
    pub s: LocalSession,
    ticks: HashMap<u32, Vec<(PlayerId, UserCmd)>>,
    end: Option<(u32, u64)>,
    last_tick: u32,
    /// The recording player's command in the latest tick.
    last_cmd: UserCmd,
    pub verdict: Option<Verdict>,
}

pub struct NetReplay {
    snaps: Vec<(u64, Snapshot)>,
    cmds: Vec<(u64, UserCmd)>,
    world: World,
    cfg: GameConfig,
    me: PlayerId,
    /// Playback clock (µs since the recording started).
    t_us: u64,
}

// One player at a time, boxed in the session: the size difference doesn't matter.
#[allow(clippy::large_enum_variant)]
pub enum DemoPlayer {
    Local(LocalReplay),
    Net(NetReplay),
}

fn load(demo: &Demo) -> Result<(World, GameConfig), String> {
    let cfg = GameConfig::from_toml_str(&demo.header.config_toml).map_err(|e| e.to_string())?;
    let p = &cfg.physics;
    let hull = (
        Vec3::from_array(p.player_mins),
        Vec3::from_array(p.player_maxs),
    );
    let world = builtin::matching(&demo.header.map, demo.header.map_hash, hull)?;
    Ok((world, cfg))
}

impl DemoPlayer {
    pub fn open(bytes: &[u8]) -> Result<DemoPlayer, String> {
        let demo = Demo::decode(bytes)?;
        let (world, cfg) = load(&demo)?;
        let me = PlayerId(demo.header.me);
        Ok(match demo.header.kind {
            DemoKind::Local => {
                let initial = Snapshot::decode(&demo.header.initial).ok_or("damaged demo start")?;
                let mut game = Game::new(cfg, world);
                initial.apply(&mut game);
                let mut ticks = HashMap::new();
                let mut last_tick = game.tick.0;
                for f in &demo.frames {
                    if let DemoFrame::Tick { tick, cmds } = f {
                        ticks.insert(
                            *tick,
                            cmds.iter().map(|(p, c)| (PlayerId(*p), *c)).collect(),
                        );
                        last_tick = last_tick.max(*tick + 1);
                    }
                }
                DemoPlayer::Local(LocalReplay {
                    s: LocalSession::from_game(game, me),
                    ticks,
                    end: demo.end(),
                    last_tick,
                    last_cmd: UserCmd::default(),
                    verdict: None,
                })
            }
            DemoKind::Net => {
                let mut snaps = Vec::new();
                let mut cmds = Vec::new();
                for f in &demo.frames {
                    match f {
                        DemoFrame::Snapshot { at_us, data } => {
                            if let Some(s) = Snapshot::decode(data) {
                                snaps.push((*at_us, s));
                            }
                        }
                        DemoFrame::Cmd { at_us, cmd } => cmds.push((*at_us, *cmd)),
                        _ => {}
                    }
                }
                if snaps.is_empty() {
                    return Err("the demo has no snapshots".into());
                }
                DemoPlayer::Net(NetReplay {
                    snaps,
                    cmds,
                    world,
                    cfg,
                    me,
                    t_us: 0,
                })
            }
        })
    }

    /// Play `elapsed_us` of real time.
    pub fn advance(&mut self, elapsed_us: u64) {
        match self {
            DemoPlayer::Local(r) => {
                for _ in 0..r.s.due_ticks(elapsed_us) {
                    if !r.step() {
                        break;
                    }
                }
            }
            DemoPlayer::Net(n) => n.t_us += elapsed_us,
        }
    }

    pub fn finished(&self) -> bool {
        match self {
            DemoPlayer::Local(r) => r.verdict.is_some(),
            DemoPlayer::Net(n) => n.t_us > n.snaps.last().map_or(0, |s| s.0),
        }
    }

    pub fn world_cfg(&self) -> (&World, &GameConfig) {
        match self {
            DemoPlayer::Local(r) => (&r.s.game.world, &r.s.game.cfg),
            DemoPlayer::Net(n) => (&n.world, &n.cfg),
        }
    }

    /// The recording player's view (their own angles, not the mouse).
    pub fn frame_view(&self) -> Option<FrameView> {
        match self {
            DemoPlayer::Local(r) => {
                let mut fv = r.s.frame_view();
                let p = r.s.game.player(r.s.me)?;
                fv.spectate = Some(SpectateView {
                    pitch: p.pitch,
                    yaw: p.yaw,
                    firing: p.buttons & buttons::FIRE != 0 && p.alive,
                    buttons: p.buttons,
                });
                fv.status = Some(match &r.verdict {
                    None => "DEMO".to_string(),
                    Some(Verdict::Match(_)) => "DEMO END - REPLAY MATCHES THE RECORDING".into(),
                    Some(Verdict::Mismatch { .. }) => {
                        "DEMO END - REPLAY DIFFERS FROM THE RECORDING".into()
                    }
                    Some(Verdict::Unverified) => "DEMO END".into(),
                });
                Some(fv)
            }
            DemoPlayer::Net(n) => n.frame_view(),
        }
    }

    /// Input overlay: what the recording player pressed.
    pub fn input_view(&self) -> InputView {
        let c = match self {
            DemoPlayer::Local(r) => r.last_cmd,
            DemoPlayer::Net(n) => n
                .cmds
                .iter()
                .rev()
                .find(|(t, _)| *t <= n.t_us)
                .map_or(UserCmd::default(), |(_, c)| *c),
        };
        InputView {
            forward: c.forward > 0,
            back: c.forward < 0,
            right: c.right > 0,
            left: c.right < 0,
            jump: c.pressed(buttons::JUMP),
            crouch: c.pressed(buttons::CROUCH),
            fire: c.pressed(buttons::FIRE),
        }
    }
}

impl LocalReplay {
    /// One recorded tick; false (and a verdict) once the recording ends.
    fn step(&mut self) -> bool {
        let t = self.s.game.tick.0;
        if t >= self.last_tick {
            if self.verdict.is_none() {
                let replayed = self.s.game.state_hash();
                self.verdict = Some(match self.end {
                    Some((tick, recorded)) if tick == t && recorded == replayed => {
                        Verdict::Match(replayed)
                    }
                    Some((_, recorded)) => Verdict::Mismatch { recorded, replayed },
                    None => Verdict::Unverified,
                });
            }
            return false;
        }
        let cmds = self.ticks.get(&t).cloned().unwrap_or_default();
        if let Some((_, c)) = cmds.iter().find(|(p, _)| *p == self.s.me) {
            self.last_cmd = *c;
        }
        self.s.step_with_cmds(&cmds);
        true
    }

    /// Re-simulate everything left as fast as possible.
    pub fn run_to_end(&mut self) -> Verdict {
        while self.step() {}
        self.verdict.clone().unwrap()
    }
}

impl NetReplay {
    fn frame_view(&self) -> Option<FrameView> {
        let i = self.snaps.partition_point(|(t, _)| *t <= self.t_us).max(1) - 1;
        let snap = &self.snaps[i].1;
        // The recording player, else whoever is first (they may have been spectating).
        let me = snap.player(self.me).or(snap.players.first())?;
        let ph = &self.cfg.physics;
        let h = if me.pm.ducked {
            ph.crouch_view_height
        } else {
            ph.view_height
        };
        let others = snap
            .players
            .iter()
            .filter(|p| p.id != me.id)
            .map(|p| OtherView {
                id: p.id,
                origin: p.pm.origin,
                pitch: p.pitch,
                yaw: p.yaw,
                ducked: p.pm.ducked,
                alive: p.alive,
                buttons: p.buttons,
            })
            .collect();
        let rate = self.cfg.tick_rate_hz as f32;
        Some(FrameView {
            me: me.id,
            me_alive: me.alive,
            eye: me.pm.origin + Vec3::new(0.0, 0.0, h),
            others,
            hud: crate::hud::HudInfo {
                health: me.health,
                accuracy_pct: me.stats.accuracy_pct(),
                damage_dealt: me.stats.damage_dealt,
                damage_taken: me.stats.damage_taken,
                wins: me.stats.fights_won,
                losses: me.stats.fights_lost,
                fight_secs: snap
                    .fight_start
                    .map(|s| snap.tick.0.saturating_sub(s.0) as f32 / rate),
                opponent_health: snap
                    .players
                    .iter()
                    .find(|p| p.id != me.id)
                    .map(|p| p.health),
                net: None,
            },
            events: vec![],
            spectate: Some(SpectateView {
                pitch: me.pitch,
                yaw: me.yaw,
                firing: me.buttons & buttons::FIRE != 0 && me.alive,
                buttons: me.buttons,
            }),
            status: Some("DEMO (ONLINE)".into()),
            stats: snap.players.iter().map(|p| (p.id, p.stats)).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgduel_bot::{AimSkill, Bot, BotConfig, StrafePattern};
    use lgduel_sim::math::{Angle, look_angles};

    /// Record a practice duel (own player aims at the bot and strafes), then
    /// replay it: the re-simulated end state must hash exactly the same.
    #[test]
    fn local_demo_replays_bit_exactly() {
        let cfg = GameConfig::default();
        let p = &cfg.physics;
        let hull = (
            Vec3::from_array(p.player_mins),
            Vec3::from_array(p.player_maxs),
        );
        let world = builtin::load("arena_pillars", hull).unwrap();
        let mut s = LocalSession::new(cfg, world, 1);
        let bot_id = s.game.players[1].id;
        let mut bot = Bot::new(BotConfig {
            pattern: StrafePattern::RandomAd {
                min_ms: 150,
                max_ms: 600,
            },
            aim: AimSkill::HARD,
            seed: 3,
            ..BotConfig::default()
        });
        s.start_recording();
        for t in 0..3_000u32 {
            let me = s.game.player(s.me).unwrap();
            let target = s.game.player(bot_id).unwrap().pm.origin;
            let (pitch, yaw) = look_angles(target - me.pm.origin);
            let mine = UserCmd {
                tick: s.game.tick,
                pitch,
                yaw: Angle(yaw.0.wrapping_add((t % 50) * 1_000_000)),
                forward: 0,
                right: if (t / 60) % 2 == 0 { 127 } else { -127 },
                buttons: buttons::FIRE | if t % 97 == 0 { buttons::JUMP } else { 0 },
            };
            let theirs = bot.think(&s.game, bot_id);
            s.step_with_cmds(&[(bot_id, theirs), (s.me, mine)]);
        }
        let stats = s.game.player(s.me).unwrap().stats;
        assert!(
            stats.damage_dealt > 0 && stats.damage_taken > 0,
            "a real fight: {stats:?}"
        );
        let demo = s.stop_recording().unwrap();
        let (end_tick, hash) = demo.end().unwrap();
        assert_eq!(end_tick, 3_000);
        let bytes = demo.encode();
        let DemoPlayer::Local(mut r) = DemoPlayer::open(&bytes).unwrap() else {
            panic!("local demo");
        };
        assert_eq!(r.run_to_end(), Verdict::Match(hash));
        assert_eq!(r.s.game.player(r.s.me).unwrap().stats, stats);
        // A tampered command changes the outcome and is detected.
        let mut bad = demo.clone();
        for f in &mut bad.frames {
            if let DemoFrame::Tick { tick: 100, cmds } = f {
                cmds[1].1.yaw = Angle(cmds[1].1.yaw.0.wrapping_add(1 << 30));
            }
        }
        let DemoPlayer::Local(mut r) = DemoPlayer::open(&bad.encode()).unwrap() else {
            panic!();
        };
        assert!(matches!(r.run_to_end(), Verdict::Mismatch { .. }));
    }
}
