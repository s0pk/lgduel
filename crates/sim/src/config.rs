//! Gameplay configuration shared by client and server.
//!
//! Every gameplay number lives here with its default. Values and their sources
//! are documented in `STATE/DECISIONS.md` (D3). Parsing a TOML string is pure,
//! so this stays inside the I/O-free sim crate; reading files is the caller's job.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Top-level gameplay config. Missing fields take their defaults; unknown
/// fields are rejected so typos in a server config surface immediately.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GameConfig {
    /// Fixed simulation rate in Hz.
    pub tick_rate_hz: u32,
    pub physics: PhysicsConfig,
    pub lg: LightningGunConfig,
    pub rules: RulesConfig,
}

/// Player movement constants (Quake units, seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysicsConfig {
    /// Downward acceleration, u/s².
    pub gravity: f32,
    /// Max wish speed on the ground, u/s.
    pub max_speed: f32,
    /// Vertical velocity set on jump, u/s.
    pub jump_velocity: f32,
    /// Ground acceleration factor.
    pub accelerate: f32,
    /// Air acceleration factor.
    pub air_accelerate: f32,
    /// Ground friction factor.
    pub friction: f32,
    /// Below this speed friction acts as if moving at this speed, u/s.
    pub stop_speed: f32,
    /// Max height the player steps up without jumping, u.
    pub step_height: f32,
    /// Wish-speed multiplier while crouched.
    pub crouch_speed_scale: f32,
    /// Standing hull mins (x, y, z) relative to origin.
    pub player_mins: [f32; 3],
    /// Standing hull maxs (x, y, z) relative to origin.
    pub player_maxs: [f32; 3],
    /// Hull maxs.z while crouched.
    pub crouch_maxs_z: f32,
}

/// Lightning Gun (hitscan beam).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LightningGunConfig {
    /// Time between hits, ms.
    pub refire_ms: u32,
    /// Damage per hit.
    pub damage: u32,
    /// Beam range, u.
    pub range: f32,
    /// Knockback scale applied per hit (0 = none).
    pub knockback: f32,
}

/// Duel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DuelMode {
    /// Lifesteal duel: damage dealt heals the attacker.
    Vamp,
    /// Plain first-to-N frags.
    Classic,
}

/// Hit resolution flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitMode {
    /// Server rewinds targets to the shooter's view time.
    AntiLag,
    /// Hits resolve against current server state.
    NoAntiLag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RulesConfig {
    pub mode: DuelMode,
    /// Percent of damage dealt returned to the attacker as health (0–100).
    pub lifesteal_percent: u32,
    /// Frags to win a classic duel.
    pub frag_limit: u32,
    pub spawn_health: u32,
    /// Health cap; lifesteal never heals above this.
    pub max_health: u32,
    /// Armor on spawn (0 = armor disabled).
    pub spawn_armor: u32,
    pub respawn_delay_ms: u32,
    pub hit_mode: HitMode,
    /// Max rewind for anti-lag, ms.
    pub max_rewind_ms: u32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            tick_rate_hz: crate::DEFAULT_TICK_RATE_HZ,
            physics: PhysicsConfig::default(),
            lg: LightningGunConfig::default(),
            rules: RulesConfig::default(),
        }
    }
}

impl Default for PhysicsConfig {
    fn default() -> Self {
        Self {
            gravity: 800.0,
            max_speed: 320.0,
            jump_velocity: 275.0,
            accelerate: 10.0,
            air_accelerate: 1.0,
            friction: 6.0,
            stop_speed: 100.0,
            step_height: 18.0,
            crouch_speed_scale: 0.25,
            player_mins: [-15.0, -15.0, -24.0],
            player_maxs: [15.0, 15.0, 32.0],
            crouch_maxs_z: 16.0,
        }
    }
}

impl Default for LightningGunConfig {
    fn default() -> Self {
        Self {
            refire_ms: 50,
            damage: 7,
            range: 768.0,
            knockback: 1.0,
        }
    }
}

impl Default for RulesConfig {
    fn default() -> Self {
        Self {
            mode: DuelMode::Vamp,
            lifesteal_percent: 100,
            frag_limit: 10,
            spawn_health: 100,
            max_health: 100,
            spawn_armor: 0,
            respawn_delay_ms: 1000,
            hit_mode: HitMode::AntiLag,
            max_rewind_ms: 200,
        }
    }
}

/// A config that parsed but violates an invariant, or failed to parse.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigError {
    Parse(String),
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Parse(m) => write!(f, "config parse error: {m}"),
            ConfigError::Invalid(m) => write!(f, "invalid config: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl GameConfig {
    /// Parse and validate a TOML document.
    pub fn from_toml_str(s: &str) -> Result<Self, ConfigError> {
        let cfg: GameConfig = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Serialize to TOML.
    pub fn to_toml_string(&self) -> String {
        toml::to_string(self).expect("GameConfig serializes to TOML")
    }

    /// Check invariants the sim relies on.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let bad = |m: String| Err(ConfigError::Invalid(m));
        if !(10..=1000).contains(&self.tick_rate_hz) {
            return bad(format!(
                "tick_rate_hz {} not in 10..=1000",
                self.tick_rate_hz
            ));
        }
        let p = &self.physics;
        let non_negative = [
            ("gravity", p.gravity),
            ("max_speed", p.max_speed),
            ("jump_velocity", p.jump_velocity),
            ("accelerate", p.accelerate),
            ("air_accelerate", p.air_accelerate),
            ("friction", p.friction),
            ("stop_speed", p.stop_speed),
            ("step_height", p.step_height),
            ("crouch_speed_scale", p.crouch_speed_scale),
            ("lg.range", self.lg.range),
            ("lg.knockback", self.lg.knockback),
        ];
        for (name, v) in non_negative {
            if !v.is_finite() || v < 0.0 {
                return bad(format!("{name} must be finite and >= 0, got {v}"));
            }
        }
        for axis in 0..3 {
            if p.player_mins[axis] >= p.player_maxs[axis] {
                return bad(format!("player_mins[{axis}] must be < player_maxs[{axis}]"));
            }
        }
        if p.crouch_maxs_z <= p.player_mins[2] || p.crouch_maxs_z > p.player_maxs[2] {
            return bad("crouch_maxs_z must be in (player_mins.z, player_maxs.z]".into());
        }
        if self.lg.refire_ms == 0 {
            return bad("lg.refire_ms must be > 0".into());
        }
        let r = &self.rules;
        if r.lifesteal_percent > 100 {
            return bad(format!("lifesteal_percent {} > 100", r.lifesteal_percent));
        }
        if r.spawn_health == 0 || r.spawn_health > r.max_health {
            return bad("spawn_health must be in 1..=max_health".into());
        }
        if r.mode == DuelMode::Classic && r.frag_limit == 0 {
            return bad("frag_limit must be > 0 in classic mode".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_vamp() {
        let c = GameConfig::default();
        c.validate().unwrap();
        assert_eq!(c.rules.mode, DuelMode::Vamp);
        assert_eq!(c.rules.lifesteal_percent, 100);
        assert_eq!(c.tick_rate_hz, 125);
    }

    #[test]
    fn round_trip_is_identical() {
        let c = GameConfig::default();
        let back = GameConfig::from_toml_str(&c.to_toml_string()).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn empty_document_gives_defaults() {
        assert_eq!(
            GameConfig::from_toml_str("").unwrap(),
            GameConfig::default()
        );
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let c = GameConfig::from_toml_str(
            "[rules]\nmode = \"classic\"\nlifesteal_percent = 0\nhit_mode = \"no_anti_lag\"\n",
        )
        .unwrap();
        assert_eq!(c.rules.mode, DuelMode::Classic);
        assert_eq!(c.rules.hit_mode, HitMode::NoAntiLag);
        assert_eq!(c.rules.lifesteal_percent, 0);
        assert_eq!(c.physics, PhysicsConfig::default());
        assert_eq!(c.rules.frag_limit, 10);
    }

    #[test]
    fn example_config_matches_defaults() {
        let text = include_str!("../../../config/server.example.toml");
        assert_eq!(
            GameConfig::from_toml_str(text).unwrap(),
            GameConfig::default()
        );
    }

    #[test]
    fn rejects_unknown_field() {
        let e = GameConfig::from_toml_str("[physics]\ngravty = 800.0\n").unwrap_err();
        assert!(matches!(e, ConfigError::Parse(_)), "{e}");
    }

    #[test]
    fn rejects_invalid_values() {
        for doc in [
            "[rules]\nlifesteal_percent = 101\n",
            "tick_rate_hz = 0\n",
            "[lg]\nrefire_ms = 0\n",
            "[physics]\ngravity = -1.0\n",
            "[physics]\nfriction = nan\n",
            "[rules]\nspawn_health = 150\n",
            "[physics]\ncrouch_maxs_z = 40.0\n",
        ] {
            let e = GameConfig::from_toml_str(doc).unwrap_err();
            assert!(matches!(e, ConfigError::Invalid(_)), "{doc}: {e}");
        }
    }
}
