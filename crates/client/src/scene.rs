//! What to draw this frame, independent of where the game runs (offline sim or
//! network): the local player's eye, remote players, HUD numbers and events.

use crate::effects::{self, EffectSettings};
use crate::hud::{self, DamageNumbers, HudInfo, HudSettings};
use crate::models::{self, ModelPose, ModelSettings};
use crate::players::EnemyColors;
use crate::render::{self, Camera, Vertex};
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
    /// The watched player's buttons (jump / crouch / fire for the input display).
    pub buttons: u8,
}

/// The camera for a frame: the view eye and the player's own view angles, or
/// the watched player's. Nothing else moves it: no screen shake or kick on
/// damage, frags or firing (visibility first; asserted in tests).
pub fn view_camera(
    fv: &FrameView,
    look_pitch: Angle,
    look_yaw: Angle,
    fov: f32,
) -> crate::render::Camera {
    crate::render::Camera {
        eye: fv.eye,
        pitch: fv.spectate.map_or(look_pitch, |s| s.pitch),
        yaw: fv.spectate.map_or(look_yaw, |s| s.yaw),
        fov,
    }
}

/// Simple LG model in the lower right of the view: barrel + body, flat shaded.
fn push_gun(v: &mut Vec<Vertex>, cam: &Camera, gun: effects::GunModel) {
    let scale = match gun {
        effects::GunModel::None => return,
        effects::GunModel::Small => 0.45,
        effects::GunModel::Normal => 0.7,
    };
    let (f, r, u) = angle_vectors(cam.pitch, cam.yaw);
    let (mf, mr, md) = effects::MUZZLE;
    let at = |fwd: f32, right: f32, down: f32| cam.eye + f * fwd + r * right - u * down;
    // (centre forward, half length, half width, colour)
    let parts = [
        (
            mf - 4.0 * scale,
            4.0 * scale,
            0.9 * scale,
            [0.30, 0.32, 0.36],
        ),
        (
            mf - 11.0 * scale,
            4.0 * scale,
            1.8 * scale,
            [0.22, 0.24, 0.28],
        ),
    ];
    for (cf, hl, hw, color) in parts {
        let c = at(cf, mr, md);
        render::push_obox(v, c, (f, r, u), Vec3::new(hl, hw, hw), color, true);
    }
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
    models: &ModelSettings,
    fx: &EffectSettings,
    seed: u32,
) -> Vec<Vertex> {
    let mut v = Vec::new();
    for o in fv.others.iter().filter(|o| o.alive) {
        let pose = ModelPose {
            origin: o.origin,
            yaw: o.yaw,
            pitch: o.pitch,
            ducked: o.ducked,
        };
        models::push_model(&mut v, models.enemy, pose, &cfg.physics, colors, models);
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
    if fv.me_alive {
        push_gun(&mut v, cam, fx.gun);
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
    input: Option<hud::InputView>,
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
    if s.input_display
        && let Some(i) = input
    {
        hud::input_display(&mut v, screen, i, s);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Camera;
    use lgduel_sim::world::builtin;

    fn setup() -> (World, GameConfig, FrameView) {
        let cfg = GameConfig::default();
        let p = &cfg.physics;
        let world = builtin::load(
            "arena_flat",
            (
                Vec3::from_array(p.player_mins),
                Vec3::from_array(p.player_maxs),
            ),
        )
        .unwrap();
        let fv = FrameView {
            me: PlayerId(0),
            me_alive: true,
            eye: Vec3::new(-300.0, 0.0, 50.0),
            others: vec![OtherView {
                id: PlayerId(1),
                origin: Vec3::new(100.0, 0.0, 24.0),
                pitch: Angle::ZERO,
                yaw: Angle::from_degrees(180.0),
                ducked: false,
                alive: true,
                buttons: buttons::FIRE,
            }],
            hud: HudInfo::default(),
            events: vec![],
            spectate: None,
            status: None,
            stats: vec![],
        };
        (world, cfg, fv)
    }

    fn violent(fv: &FrameView) -> FrameView {
        let mut f = fv.clone();
        f.events = vec![
            GameEvent::Damage {
                attacker: PlayerId(1),
                target: PlayerId(0),
                amount: 7,
                healed: 7,
            },
            GameEvent::Damage {
                attacker: PlayerId(0),
                target: PlayerId(1),
                amount: 7,
                healed: 7,
            },
            GameEvent::Kill {
                killer: PlayerId(0),
                victim: PlayerId(1),
            },
        ];
        f
    }

    #[test]
    fn no_screen_shake_and_no_blood() {
        let (world, cfg, fv) = setup();
        let hurt = violent(&fv);
        let (p, y) = (Angle::from_degrees(-5.0), Angle::from_degrees(3.0));
        // The camera never moves because of damage, frags or firing.
        assert_eq!(
            view_camera(&fv, p, y, 110.0),
            view_camera(&hurt, p, y, 110.0)
        );
        // Hits and frags add no geometry at all (no blood, gibs or screen flashes):
        // the dynamic scene is identical with and without them, gun drawn or not.
        let cam = view_camera(&fv, p, y, 110.0);
        for gun in [effects::GunModel::None, effects::GunModel::Normal] {
            let fx = EffectSettings {
                gun,
                ..EffectSettings::default()
            };
            let colors = EnemyColors::default();
            let a = scene_mesh(
                &fv,
                &world,
                &cfg,
                &cam,
                true,
                &colors,
                &ModelSettings::default(),
                &fx,
                1,
            );
            let b = scene_mesh(
                &hurt,
                &world,
                &cfg,
                &cam,
                true,
                &colors,
                &ModelSettings::default(),
                &fx,
                1,
            );
            assert_eq!(a, b);
            assert!(!a.is_empty());
        }
    }

    #[test]
    fn gun_model_options() {
        let (world, cfg, fv) = setup();
        let cam = Camera {
            eye: fv.eye,
            pitch: Angle::ZERO,
            yaw: Angle::ZERO,
            fov: 110.0,
        };
        let count = |gun| {
            let fx = EffectSettings {
                gun,
                ..EffectSettings::default()
            };
            scene_mesh(
                &fv,
                &world,
                &cfg,
                &cam,
                false,
                &EnemyColors::default(),
                &ModelSettings::default(),
                &fx,
                1,
            )
            .len()
        };
        let none = count(effects::GunModel::None);
        assert_eq!(count(effects::GunModel::Small), none + 72, "two boxes");
        assert_eq!(count(effects::GunModel::Normal), none + 72);
        // A dead player sees no gun (compared with gun off, also dead: remote
        // beams no longer hit a dead player, so the rest of the scene differs).
        let mut dead = fv.clone();
        dead.me_alive = false;
        let fx_none = EffectSettings::default();
        let none = scene_mesh(
            &dead,
            &world,
            &cfg,
            &cam,
            false,
            &EnemyColors::default(),
            &ModelSettings::default(),
            &fx_none,
            1,
        )
        .len();
        let fx = EffectSettings {
            gun: effects::GunModel::Normal,
            ..EffectSettings::default()
        };
        let d = scene_mesh(
            &dead,
            &world,
            &cfg,
            &cam,
            false,
            &EnemyColors::default(),
            &ModelSettings::default(),
            &fx,
            1,
        );
        assert_eq!(d.len(), none);
    }
}
