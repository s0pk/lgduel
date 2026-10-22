//! Server administration: console commands (typed into the server's terminal
//! or sent over RCON) and live config changes.

use crate::{Server, ServerConfig};
use lgduel_net::protocol::{Control, MAX_RCON_REPLY};
use lgduel_net::transport::Transport;
use std::net::SocketAddr;

const HELP: &str = "commands:
  status                    players, arenas, ping
  kick <name|id> [reason]   disconnect a player
  say <text>                message to everyone
  get <section.key>         show a setting, e.g. get rules.lifesteal_percent
  set <section.key> <value> change a setting now, e.g. set rules.lifesteal_percent 50
  reload                    re-read the config file
  help";

/// Wrong RCON passwords allowed per IP before it is ignored for a minute.
pub const RCON_MAX_FAILURES: u32 = 5;
const RCON_LOCKOUT_US: u64 = 60_000_000;

impl<T: Transport> Server<T> {
    /// The running configuration (all arenas share the gameplay config).
    pub fn config(&self) -> ServerConfig {
        ServerConfig {
            server: self.settings.clone(),
            game: self.game().cfg.clone(),
        }
    }

    /// Run one console command and return what to print.
    pub fn command(&mut self, line: &str) -> String {
        let line = line.trim();
        let (cmd, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match cmd {
            "" => String::new(),
            "help" => HELP.to_string(),
            "status" => self.status_text(),
            "kick" => {
                let (who, reason) = rest.split_once(' ').unwrap_or((rest, "kicked"));
                let Some(i) = self.clients.iter().position(|c| {
                    c.name.eq_ignore_ascii_case(who) || who.parse() == Ok(c.player.0)
                }) else {
                    return format!("no player {who:?}");
                };
                let reason = format!("kicked: {}", reason.trim());
                let c = &mut self.clients[i];
                c.conn.send_reliable(
                    Control::Disconnect {
                        reason: reason.clone(),
                    }
                    .encode(),
                );
                let pkt = c.conn.build_packet(self.now_us, &[]);
                let _ = self.transport.send_to(c.addr, &pkt);
                let name = c.name.clone();
                self.drop_client(i, &reason);
                format!("kicked {name}")
            }
            "say" if !rest.is_empty() => {
                for i in 0..self.clients.len() {
                    self.tell(i, format!("SERVER: {}", rest.to_uppercase()));
                }
                self.log.push(format!("say: {rest}"));
                "sent".into()
            }
            "get" => match self.config_table().and_then(|t| lookup(&t, rest).cloned()) {
                Some(v) => format!("{rest} = {v}"),
                None => format!("no setting {rest:?} (form: section.key)"),
            },
            "set" => {
                let Some((key, value)) = rest.split_once(' ') else {
                    return "usage: set <section.key> <value>".into();
                };
                match self.set(key, value.trim()) {
                    Ok(()) => {
                        self.log.push(format!("set {key} = {}", value.trim()));
                        format!("{key} = {}", value.trim())
                    }
                    Err(e) => format!("not changed: {e}"),
                }
            }
            "reload" => {
                let Some(path) = self.config_path.clone() else {
                    return "no config file (start the server with --config FILE)".into();
                };
                let result = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{path}: {e}"))
                    .and_then(|t| ServerConfig::from_toml_str(&t).map_err(|e| e.to_string()))
                    .and_then(|mut cfg| {
                        // Command-line overrides (port, bind, map) stay as started.
                        cfg.server.bind = self.settings.bind.clone();
                        cfg.server.port = self.settings.port;
                        cfg.server.map = self.settings.map.clone();
                        self.apply_config(cfg)
                    });
                match result {
                    Ok(()) => {
                        self.log.push(format!("reloaded {path}"));
                        format!("reloaded {path}")
                    }
                    Err(e) => format!("not reloaded: {e}"),
                }
            }
            _ => format!("unknown command {cmd:?}; try help"),
        }
    }

    fn status_text(&self) -> String {
        let g = self.game();
        let mut out = format!(
            "{} | map {} | tick {} | {} of {} players | {} arenas\n",
            self.settings.name,
            g.world.name,
            g.tick.0,
            self.clients.len(),
            self.settings.max_clients,
            self.arenas.len()
        );
        for c in &self.clients {
            let place = match (c.arena, c.spectate) {
                (Some(a), _) => format!("arena {}", a + 1),
                (None, true) => "spectating".into(),
                (None, false) => "queued".into(),
            };
            out.push_str(&format!(
                "  #{:<3} {:<24} {:<10} ping {:>3.0} ms  loss {:.1}%\n",
                c.player.0,
                c.name,
                place,
                c.conn.stats.rtt_us / 1000.0,
                c.conn.stats.loss() * 100.0
            ));
        }
        out
    }

    /// The running config as a TOML table (`[server]` + gameplay sections).
    fn config_table(&self) -> Option<toml::Table> {
        let cfg = self.config();
        let mut t: toml::Table = toml::from_str(&cfg.game.to_toml_string()).ok()?;
        t.insert("server".into(), toml::Value::try_from(&cfg.server).ok()?);
        Some(t)
    }

    /// Change one setting (validated like a config file) and apply it.
    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let mut t = self.config_table().ok_or("config not representable")?;
        let (section, field) = key.split_once('.').ok_or("form: section.key")?;
        let old = lookup(&t, key).ok_or(format!("no setting {key:?}"))?;
        // TOML literal if it parses as one (numbers, booleans, "strings"), else a bare string.
        let parsed = toml::from_str::<toml::Table>(&format!("v = {value}"))
            .ok()
            .and_then(|mut v| v.remove("v"))
            .unwrap_or_else(|| toml::Value::String(value.to_string()));
        // Integers given for a float setting (gravity = 800) are fine.
        let parsed = match (old, parsed) {
            (toml::Value::Float(_), toml::Value::Integer(i)) => toml::Value::Float(i as f64),
            (_, v) => v,
        };
        t.get_mut(section)
            .and_then(|s| s.as_table_mut())
            .ok_or(format!("no section {section:?}"))?
            .insert(field.to_string(), parsed);
        let text = toml::to_string(&t).map_err(|e| e.to_string())?;
        let cfg = ServerConfig::from_toml_str(&text).map_err(|e| e.to_string())?;
        self.apply_config(cfg)
    }

    /// Switch to `cfg` without a restart. Physics, tick rate, map, address
    /// and arena count are fixed for the server's lifetime (connected
    /// clients predict with the physics they got on connect).
    pub fn apply_config(&mut self, cfg: ServerConfig) -> Result<(), String> {
        let cur = self.config();
        if cfg.game.physics != cur.game.physics || cfg.game.tick_rate_hz != cur.game.tick_rate_hz {
            return Err("physics and tick rate need a restart".into());
        }
        let s = (&cfg.server, &cur.server);
        if s.0.map != s.1.map || s.0.bind != s.1.bind || s.0.port != s.1.port {
            return Err("map, bind and port need a restart".into());
        }
        if s.0.max_arenas != s.1.max_arenas {
            return Err("max_arenas needs a restart".into());
        }
        for a in &mut self.arenas {
            a.game.cfg = cfg.game.clone();
        }
        self.settings = cfg.server;
        Ok(())
    }

    /// Handle an RCON request: the reply text, or None to stay silent (RCON
    /// off, wrong password, or this address is locked out).
    pub(crate) fn rcon(
        &mut self,
        from: SocketAddr,
        password: &str,
        command: &str,
    ) -> Option<String> {
        if self.settings.rcon_password.is_empty() {
            return None;
        }
        let now = self.now_us;
        let ip = from.ip();
        if let Some((n, since)) = self.rcon_failures.get(&ip).copied() {
            if now.saturating_sub(since) > RCON_LOCKOUT_US {
                self.rcon_failures.remove(&ip);
            } else if n >= RCON_MAX_FAILURES {
                return None;
            }
        }
        if password != self.settings.rcon_password {
            let e = self.rcon_failures.entry(ip).or_insert((0, now));
            e.0 += 1;
            if e.0 == RCON_MAX_FAILURES {
                self.log.push(format!("rcon: {ip} locked out for 60 s"));
            }
            return None;
        }
        self.log.push(format!("rcon from {from}: {command}"));
        let mut text = self.command(command);
        if text.len() > MAX_RCON_REPLY {
            let mut end = MAX_RCON_REPLY - 3;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("...");
        }
        Some(text)
    }
}

/// `section.key` in a config table.
fn lookup<'a>(t: &'a toml::Table, key: &str) -> Option<&'a toml::Value> {
    let (section, field) = key.split_once('.')?;
    t.get(section)?.as_table()?.get(field)
}
