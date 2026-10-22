//! What to draw this frame, independent of where the game runs (offline sim or
//! network): the local player's eye, remote players, HUD numbers and events.

use crate::effects::{self, EffectSettings};
use crate::hud::{self, DamageNumbers, HudInfo, HudSettings};
use crate::players::{self, EnemyColors};
use crate::render::{Camera, Vertex};
use lgduel_sim::config::GameConfig;
use lgduel_sim::game::{GameEvent, PlayerId, hitscan};
use lgduel_sim::math::{Angle, Vec3, angle_vectors};
use lgduel_sim::usercmd::buttons;
use lgduel_sim::world::{Aabb, World};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OtherView {
    pub id: PlayerId,
    pub origin: Vec3,
    pub pitch: Angle,
    pub yaw: Angle,
    pub ducked: bool,
    pub alive: bool,
    pub buttons: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FrameView {
    pub me: PlayerId,
    pub me_alive: bool,
    /// Own eye position to render from (interpolated / predicted).
    pub eye: Vec3,
    pub others: Vec<OtherView>,
    pub hud: HudInfo,
    /// Game events since the previous frame.
    pub events: Vec<GameEvent>,
    /// Set while watching someone else (`me` is then the watched player):
    /// the camera uses their view angles instead of the mouse.
    pub spectate: Option<SpectateView>,
    /// One line at the top of the screen (arena / queue / spectating).
    pub status: Option<String>,
    /// Everyone's running stats (for the end-of-fight summary).
    pub stats: Vec<(PlayerId, lgduel_sim::game::PlayerStats)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectateView {
    pub pitch: Angle,
    pub yaw: Angle,
    pub firing: bool,
}

impl FrameView {
    fn hitbox(o: &OtherView, cfg: &GameConfig) -> Aabb {
        let mins = Vec3::from_array(cfg.physics.player_mins);
        let mut maxs = Vec3::from_array(cfg.physics.player_maxs);
        if o.ducked {
            maxs.z = cfg.physics.crouch_maxs_z;
        }
        Aabb::at(o.origin, mins, maxs)
    }

    /// Eye of a remote player.
    fn eye_of(o: &OtherView, cfg: &GameConfig) -> Vec3 {
        let h = if o.ducked {
            cfg.physics.crouch_view_height
        } else {
            cfg.physics.view_height
        };
        o.origin + Vec3::new(0.0, 0.0, h)
    }
}

/// Beam from `eye` along (pitch, yaw), drawn from a muzzle offset; sparks where it hits.
#[allow(clippy::too_many_arguments)]
fn push_lg(
    v: &mut Vec<Vertex>,
    fv: &FrameView,
    world: &World,
    cfg: &GameConfig,
    shooter: PlayerId,
    eye: Vec3,
    pitch: Angle,
    yaw: Angle,
    view_eye: Vec3,
    fx: &EffectSettings,
    seed: u32,
) {
    let mut targets: Vec<(PlayerId, Aabb)> = fv
        .others
        .iter()
        .filter(|o| o.alive && o.id != shooter)
        .map(|o| (o.id, FrameView::hitbox(o, cfg)))
        .collect();
    if shooter != fv.me && fv.me_alive {
        // Remote beams can hit us; our hull at our (rendered) feet.
        let feet = fv.eye - Vec3::new(0.0, 0.0, cfg.physics.view_height);
        let me = OtherView {
            id: fv.me,
            origin: feet,
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            ducked: false,
            alive: true,
            buttons: 0,
        };
        targets.push((fv.me, FrameView::hitbox(&me, cfg)));
    }
    let (f, r, u) = angle_vectors(pitch, yaw);
    let hs = hitscan(world, eye, f, cfg.lg.range, &targets);
    let (mf, mr, md) = effects::MUZZLE;
    let muzzle = eye + f * mf + r * mr - u * md;
    effects::push_beam(v, muzzle, hs.end, view_eye, fx, seed);
    if (hs.end - eye).length() < cfg.lg.range - 0.5 {
        effects::push_sparks(v, hs.end, f, view_eye, fx, seed);
    }
}

/// Dynamic geometry: opponents, their beams, and (if firing) the own beam from `cam`.
#[allow(clippy::too_many_arguments)]
pub fn scene_mesh(
    fv: &FrameView,
    world: &World,
    cfg: &GameConfig,
    cam: &Camera,
    firing: bool,
    colors: &EnemyColors,
    fx: &EffectSettings,
    seed: u32,
) -> Vec<Vertex> {
    let mut v = Vec::new();
    for o in fv.others.iter().filter(|o| o.alive) {
        players::push_player(&mut v, o.origin, o.yaw, o.ducked, &cfg.physics, colors);
        if o.buttons & buttons::FIRE != 0 {
            let eye = FrameView::eye_of(o, cfg);
            push_lg(
                &mut v, fv, world, cfg, o.id, eye, o.pitch, o.yaw, cam.eye, fx, seed,
            );
        }
    }
    if firing && fv.me_alive {
        push_lg(
            &mut v, fv, world, cfg, fv.me, cam.eye, cam.pitch, cam.yaw, cam.eye, fx, seed,
        );
    }
    v
}

/// Turn this frame's own hits into floating damage numbers; returns the hit count.
pub fn track_hits(fv: &FrameView, numbers: &mut DamageNumbers) -> usize {
    let mut last_end = None;
    let mut hits = 0;
    for e in &fv.events {
        match e {
            GameEvent::Fire { shooter, end, .. } if *shooter == fv.me => last_end = Some(*end),
            GameEvent::Damage {
                attacker, amount, ..
            } if *attacker == fv.me => {
                if let Some(p) = last_end {
                    numbers.add(p, *amount);
                }
                hits += 1;
            }
            _ => {}
        }
    }
    hits
}

pub fn hud_mesh(
    fv: &FrameView,
    cam: &Camera,
    size: (u32, u32),
    numbers: &DamageNumbers,
    s: &HudSettings,
    summary: Option<&crate::fight::FightSummary>,
) -> Vec<Vertex> {
    let mut v = Vec::new();
    let screen = (size.0 as f32, size.1 as f32);
    let vp = cam.view_proj(screen.0 / screen.1.max(1.0));
    hud::build(&mut v, screen, &vp, &fv.hud, numbers, s);
    if let Some(t) = &fv.status {
        hud::status_line(&mut v, screen, t, s);
    }
    if let Some(f) = summary {
        hud::centre_block(&mut v, screen, &f.lines(), s);
    }
    v
}
