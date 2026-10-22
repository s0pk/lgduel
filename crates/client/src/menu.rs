//! In-game menus drawn with egui: the Esc menu and the settings screen.
//! Settings edit a `Settings` value in place; the app applies changes live and
//! saves them when the settings screen is left.

use crate::browser::{Row, Status};
use crate::effects::BeamStyle;
use crate::settings::Settings;

/// Which menu screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    /// Playing; no menu.
    #[default]
    Closed,
    /// Esc menu.
    Pause,
    /// Settings (opened from the pause or main menu).
    Settings,
    /// Start screen (no game running).
    Main,
    /// Practice setup.
    Practice,
    /// Play online: server list / direct connect.
    Online,
    /// Waiting for a server to accept us.
    Connecting,
    /// Host a game from this PC.
    Host,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Mouse,
    Video,
    Visuals,
    Audio,
    Player,
}

impl Tab {
    const ALL: [Tab; 5] = [
        Tab::Mouse,
        Tab::Video,
        Tab::Visuals,
        Tab::Audio,
        Tab::Player,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Mouse => "Mouse",
            Tab::Video => "Video",
            Tab::Visuals => "Visuals",
            Tab::Audio => "Audio",
            Tab::Player => "Player",
        }
    }
}

/// What the player chose this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Resume,
    Disconnect,
    Quit,
    /// Play the hit sound once (audio test).
    TestSound,
    /// The settings screen was closed (time to save).
    SettingsClosed,
    StartPractice,
    /// Connect to this address (host[:port]).
    Connect(String),
    /// Join by code ("LG-XXXX") through the rendezvous service.
    JoinCode(String),
    CancelConnect,
    RefreshServers,
    StartHost,
    AddServer {
        name: String,
        address: String,
    },
    RemoveServer(String),
    /// Online: watch (`true`) or play (`false`).
    Spectate(bool),
    /// Online: challenge this player id.
    Challenge(u8),
    /// Online: answer the challenge from this player id.
    Respond {
        from: u8,
        accept: bool,
    },
    /// A line typed into the console.
    ConsoleLine(String),
}

/// One row of the in-game player list.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerRow {
    pub id: u8,
    pub name: String,
    /// "arena 2", "queued", "spectating".
    pub place: String,
    pub me: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Menu {
    pub screen: Screen,
    pub tab: Tab,
    /// Settings changed since the screen was opened.
    pub dirty: bool,
    /// Where "Back" from the settings screen goes.
    pub settings_return: Screen,
    /// Direct-connect text field.
    pub connect_text: String,
    /// Status / error line shown on the main and online screens.
    pub message: Option<String>,
    /// What we are connecting to (Connecting screen).
    pub connecting_to: String,
    /// Shown in the pause menu while this PC hosts the game.
    pub hosting_info: Option<String>,
    /// "Add server" form.
    pub add_name: String,
    pub add_address: String,
    /// Online: whether we are spectating (None offline); set by the app each frame.
    pub spectating: Option<bool>,
    /// Online: players on the server and a pending challenge to us (set each frame).
    pub players: Vec<PlayerRow>,
    pub invite: Option<(u8, String)>,
    /// Join-code field (Online screen) and progress line (Connecting screen).
    pub code_text: String,
    pub connect_detail: Option<String>,
    /// A rendezvous service is available (shipped or set by the player).
    pub has_rendezvous: bool,
    /// Drop-down console (` key).
    pub console: crate::console::Console,
    /// Theme names to offer (built-ins and folders); empty = built-ins only.
    pub themes: Vec<String>,
}

pub const TITLE: &str = "LGDuel";

const RESOLUTIONS: [(u32, u32); 6] = [
    (1280, 720),
    (1600, 900),
    (1920, 1080),
    (2560, 1440),
    (1024, 768),
    (1280, 1024),
];
const PATTERNS: [&str; 6] = ["still", "ad", "mirror", "anti", "hsw", "hswanti"];
const SKILLS: [&str; 4] = ["easy", "medium", "hard", "aimbot"];
const MAPS: [&str; 3] = [
    lgduel_sim::world::builtin::ALL[0].0,
    lgduel_sim::world::builtin::ALL[1].0,
    lgduel_sim::world::builtin::ALL[2].0,
];

impl Menu {
    /// A menu screen or the console is up (mouse released, keys to the UI).
    pub fn is_open(&self) -> bool {
        self.screen != Screen::Closed || self.console.open
    }

    fn open_settings(&mut self, from: Screen) {
        self.settings_return = from;
        self.screen = Screen::Settings;
        self.dirty = false;
    }

    /// Esc on a menu screen: one level back (never closes the main menu).
    pub fn back(&mut self) -> Option<Action> {
        match self.screen {
            Screen::Practice | Screen::Online | Screen::Host => self.screen = Screen::Main,
            Screen::Settings => {
                self.screen = self.settings_return;
                return Some(Action::SettingsClosed);
            }
            Screen::Connecting => return Some(Action::CancelConnect),
            _ => {}
        }
        None
    }

    /// Draw the current screen; may edit `s` (sets `dirty`) and return an action.
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        s: &mut Settings,
        online: bool,
        servers: &[Row],
    ) -> Option<Action> {
        let mut action = None;
        match self.screen {
            Screen::Closed => {}
            Screen::Pause => {
                egui::Window::new(TITLE)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .fixed_size(egui::vec2(
                        if self.players.len() > 1 { 380.0 } else { 260.0 },
                        0.0,
                    ))
                    .show(ctx, |ui| {
                        ui.vertical_centered_justified(|ui| {
                            if ui.button("Resume").clicked() {
                                action = Some(Action::Resume);
                            }
                            if ui.button("Settings").clicked() {
                                self.open_settings(Screen::Pause);
                            }
                            if let Some(sp) = self.spectating {
                                let label = if sp { "Play" } else { "Spectate" };
                                if ui.button(label).clicked() {
                                    action = Some(Action::Spectate(!sp));
                                }
                            }
                            let leave = if online {
                                "Disconnect"
                            } else {
                                "Leave practice"
                            };
                            if ui.button(leave).clicked() {
                                action = Some(Action::Disconnect);
                            }
                            if ui.button("Quit").clicked() {
                                action = Some(Action::Quit);
                            }
                        });
                        if let Some(h) = &self.hosting_info {
                            ui.add_space(6.0);
                            ui.label(h);
                        }
                        if let Some((from, name)) = &self.invite {
                            ui.add_space(6.0);
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 210, 90),
                                format!("{name} challenges you"),
                            );
                            ui.horizontal(|ui| {
                                if ui.button("Accept").clicked() {
                                    action = Some(Action::Respond {
                                        from: *from,
                                        accept: true,
                                    });
                                }
                                if ui.button("Decline").clicked() {
                                    action = Some(Action::Respond {
                                        from: *from,
                                        accept: false,
                                    });
                                }
                            });
                        }
                        if self.players.len() > 1 {
                            ui.add_space(6.0);
                            ui.separator();
                            ui.label("Players");
                            egui::Grid::new("players").striped(true).show(ui, |ui| {
                                for p in &self.players {
                                    ui.label(&p.name);
                                    ui.label(&p.place);
                                    if p.me {
                                        ui.label("(you)");
                                    } else if ui.button("Challenge").clicked() {
                                        action = Some(Action::Challenge(p.id));
                                    }
                                    ui.end_row();
                                }
                            });
                        }
                    });
            }
            Screen::Settings => {
                // Left side, so the real crosshair/HUD in the middle stays visible as a preview.
                egui::Window::new("Settings")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::LEFT_CENTER, egui::vec2(24.0, 0.0))
                    .fixed_size(egui::vec2(470.0, 470.0))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            for t in Tab::ALL {
                                ui.selectable_value(&mut self.tab, t, t.label());
                            }
                        });
                        ui.separator();
                        let before = s.clone();
                        egui::ScrollArea::vertical()
                            .max_height(380.0)
                            .show(ui, |ui| match self.tab {
                                Tab::Mouse => mouse_tab(ui, s),
                                Tab::Video => video_tab(ui, s),
                                Tab::Visuals => visuals_tab(ui, s, &self.themes),
                                Tab::Audio => {
                                    if audio_tab(ui, s) {
                                        action = Some(Action::TestSound);
                                    }
                                }
                                Tab::Player => player_tab(ui, s),
                            });
                        if *s != before {
                            s.sanitize();
                            self.dirty = true;
                        }
                        ui.separator();
                        if ui.button("Back").clicked() {
                            self.screen = self.settings_return;
                            action = Some(Action::SettingsClosed);
                        }
                    });
            }
            Screen::Main => {
                centered(ctx, TITLE, 300.0, |ui| {
                    ui.vertical_centered(|ui| ui.label("Lightning Gun duel trainer"));
                    ui.add_space(6.0);
                    ui.vertical_centered_justified(|ui| {
                        if ui.button("Practice vs bot").clicked() {
                            self.screen = Screen::Practice;
                        }
                        if ui.button("Play online").clicked() {
                            self.screen = Screen::Online;
                            self.message = None;
                        }
                        if ui.button("Host a game").clicked() {
                            self.screen = Screen::Host;
                            self.message = None;
                        }
                        if ui.button("Settings").clicked() {
                            self.open_settings(Screen::Main);
                        }
                        if ui.button("Quit").clicked() {
                            action = Some(Action::Quit);
                        }
                        if let Some(m) = &self.message {
                            ui.add_space(6.0);
                            ui.colored_label(egui::Color32::from_rgb(255, 170, 90), m);
                        }
                    });
                });
            }
            Screen::Practice => {
                let before = s.practice.clone();
                centered(ctx, "Practice", 420.0, |ui| {
                    practice_settings(ui, s);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Start").clicked() {
                            action = Some(Action::StartPractice);
                        }
                        if ui.button("Back").clicked() {
                            self.screen = Screen::Main;
                        }
                    });
                });
                if s.practice != before {
                    self.dirty = true;
                }
            }
            Screen::Online => {
                let before = (s.filter.clone(), s.rendezvous.clone());
                centered(ctx, "Play online", 900.0, |ui| {
                    filter_row(ui, &mut s.filter);
                    let shown: Vec<Row> = servers
                        .iter()
                        .filter(|r| match &r.status {
                            Status::Online(p) => s.filter.allows(&p.info, p.rtt_ms),
                            _ => true,
                        })
                        .cloned()
                        .collect();
                    let hidden = servers.len() - shown.len();
                    action = server_table(ui, &shown).or(action.take());
                    if hidden > 0 {
                        ui.colored_label(
                            egui::Color32::from_gray(130),
                            format!("{hidden} hidden by the filters"),
                        );
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Refresh").clicked() {
                            action = Some(Action::RefreshServers);
                        }
                    });
                    ui.separator();
                    ui.label("Add a server to the list");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_name)
                                .hint_text("name")
                                .desired_width(200.0),
                        );
                        ui.add(
                            egui::TextEdit::singleline(&mut self.add_address)
                                .hint_text("address:port")
                                .desired_width(260.0),
                        );
                        let ok = !self.add_address.trim().is_empty();
                        if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                            let name = if self.add_name.trim().is_empty() {
                                self.add_address.trim()
                            } else {
                                self.add_name.trim()
                            };
                            action = Some(Action::AddServer {
                                name: name.to_string(),
                                address: self.add_address.trim().to_string(),
                            });
                            self.add_name.clear();
                            self.add_address.clear();
                        }
                    });
                    ui.label("Join code (a friend hosting with Host a game; no port forwarding)");
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.code_text)
                                .hint_text("LG-7KQ2")
                                .desired_width(140.0)
                                .char_limit(8),
                        );
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let ok = !self.code_text.trim().is_empty();
                        if (ui.add_enabled(ok, egui::Button::new("Join")).clicked() || enter) && ok
                        {
                            action = Some(Action::JoinCode(self.code_text.trim().to_string()));
                        }
                        ui.label("via");
                        ui.add(
                            egui::TextEdit::singleline(&mut s.rendezvous)
                                .hint_text(if self.has_rendezvous {
                                    "official relay"
                                } else {
                                    "relay host:port"
                                })
                                .desired_width(220.0),
                        );
                    });
                    ui.label("Direct connect (address or address:port)");
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.connect_text)
                                .hint_text("1.2.3.4:27960")
                                .desired_width(300.0),
                        );
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if (ui.button("Connect").clicked() || enter)
                            && !self.connect_text.trim().is_empty()
                        {
                            action = Some(Action::Connect(self.connect_text.trim().to_string()));
                        }
                    });
                    if let Some(m) = &self.message {
                        ui.colored_label(egui::Color32::from_rgb(255, 170, 90), m);
                    }
                    ui.add_space(6.0);
                    if ui.button("Back").clicked() {
                        self.screen = Screen::Main;
                    }
                });
                if (s.filter.clone(), s.rendezvous.clone()) != before {
                    self.dirty = true;
                }
            }
            Screen::Host => {
                let before = s.host.clone();
                centered(ctx, "Host a game", 480.0, |ui| {
                    let h = &mut s.host;
                    row(ui, "Server name", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut h.name)
                                .char_limit(lgduel_net::protocol::MAX_INFO_TEXT),
                        );
                    });
                    row(ui, "Map", |ui| {
                        combo(ui, "host_map", &mut h.map, &MAPS);
                    });
                    row(ui, "UDP port", |ui| {
                        ui.add(egui::DragValue::new(&mut h.port).range(1024..=65535));
                    });
                    row(ui, "Mode", |ui| {
                        ui.selectable_value(&mut h.classic, false, "Vamp");
                        ui.selectable_value(&mut h.classic, true, "Classic");
                    });
                    row(ui, "Lifesteal %", |ui| {
                        ui.add(egui::Slider::new(&mut h.lifesteal_percent, 0..=100));
                    });
                    row(ui, "Anti-lag", |ui| {
                        ui.checkbox(&mut h.anti_lag, "");
                    });
                    row(ui, "Join code", |ui| {
                        ui.add_enabled(
                            self.has_rendezvous,
                            egui::Checkbox::new(
                                &mut h.join_code,
                                "friends need no port forwarding",
                            ),
                        );
                    });
                    ui.label(
                        if h.join_code && self.has_rendezvous {
                            "Friends on your network see this game under Play online. Anyone else joins with the join code shown in your Esc menu once hosting: no port forwarding needed."
                        } else {
                            "Friends on your network see this game under Play online. Over the internet they need your public IP with this UDP port forwarded, a VPN like Tailscale, or a join code (needs a rendezvous service: Play online -> Join code -> via)."
                        },
                    );
                    if let Some(m) = &self.message {
                        ui.colored_label(egui::Color32::from_rgb(255, 170, 90), m);
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Start hosting").clicked() {
                            action = Some(Action::StartHost);
                        }
                        if ui.button("Back").clicked() {
                            self.screen = Screen::Main;
                        }
                    });
                });
                if s.host != before {
                    self.dirty = true;
                }
            }
            Screen::Connecting => {
                centered(ctx, "Connecting", 360.0, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(format!("Connecting to {}...", self.connecting_to));
                        if let Some(d) = &self.connect_detail {
                            ui.label(d);
                        }
                        ui.add_space(6.0);
                        if ui.button("Cancel").clicked() {
                            action = Some(Action::CancelConnect);
                        }
                    });
                });
            }
        }
        if action == Some(Action::Resume) {
            self.screen = Screen::Closed;
        }
        if self.console.open {
            action = self.draw_console(ctx).or(action);
        }
        action
    }
}

/// The server list; returns Join / Remove actions.
fn filter_row(ui: &mut egui::Ui, f: &mut crate::settings::BrowserFilter) {
    ui.horizontal(|ui| {
        ui.label("Mode");
        combo(ui, "f_mode", &mut f.mode, &["any", "vamp", "classic"]);
        ui.label("Anti-lag");
        combo(ui, "f_lag", &mut f.anti_lag, &["any", "on", "off"]);
        ui.label("Max ping");
        ui.add(
            egui::DragValue::new(&mut f.max_ping)
                .range(0..=500)
                .speed(5.0)
                .custom_formatter(|v, _| {
                    if v == 0.0 {
                        "any".into()
                    } else {
                        format!("{v:.0}")
                    }
                }),
        );
        ui.label("Region");
        ui.add(
            egui::TextEdit::singleline(&mut f.region)
                .hint_text("any")
                .desired_width(70.0)
                .char_limit(lgduel_net::protocol::MAX_REGION),
        );
        ui.checkbox(&mut f.hide_empty, "Hide empty");
        ui.checkbox(&mut f.hide_full, "Hide full");
    });
}

fn server_table(ui: &mut egui::Ui, rows: &[Row]) -> Option<Action> {
    let mut action = None;
    if rows.is_empty() {
        ui.label("No servers listed.");
        return None;
    }
    egui::Grid::new("servers")
        .striped(true)
        .num_columns(9)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for h in [
                "Server", "Ping", "Players", "Map", "Mode", "Anti-lag", "Region", "", "",
            ] {
                ui.strong(h);
            }
            ui.end_row();
            for r in rows {
                let dim = egui::Color32::from_gray(130);
                match &r.status {
                    Status::Online(p) => {
                        let compatible = p.info.protocol == lgduel_net::PROTOCOL_VERSION;
                        ui.label(if p.info.name.is_empty() {
                            r.name.as_str()
                        } else {
                            p.info.name.as_str()
                        })
                        .on_hover_text(&r.address);
                        ui.label(format!("{:.0} ms", p.rtt_ms));
                        ui.label(format!("{}/{}", p.info.players, p.info.max_players));
                        ui.label(&p.info.map);
                        ui.label(if p.info.classic {
                            "Classic".to_string()
                        } else {
                            format!("Vamp {}%", p.info.lifesteal_percent)
                        });
                        ui.label(if p.info.anti_lag { "On" } else { "Off" });
                        ui.label(&p.info.region);
                        if compatible {
                            if ui.button("Join").clicked() {
                                action = Some(Action::Connect(r.address.clone()));
                            }
                        } else {
                            ui.colored_label(dim, "other version");
                        }
                    }
                    other => {
                        ui.colored_label(dim, &r.name).on_hover_text(&r.address);
                        let what = match other {
                            Status::Waiting => "...".to_string(),
                            Status::NoAnswer => "no answer".to_string(),
                            Status::BadAddress(e) => format!("bad address: {e}"),
                            Status::Online(_) => unreachable!(),
                        };
                        ui.colored_label(dim, what);
                        for _ in 0..5 {
                            ui.label("");
                        }
                        ui.label("");
                    }
                }
                if r.lan || r.public {
                    let tag = if r.lan { "LAN" } else { "PUBLIC" };
                    ui.colored_label(egui::Color32::from_gray(130), tag);
                } else if r.official {
                    ui.label("");
                } else if ui.small_button("Remove").clicked() {
                    action = Some(Action::RemoveServer(r.address.clone()));
                }
                ui.end_row();
            }
        });
    action
}

impl Menu {
    /// Quake-style console across the top 40% of the screen.
    fn draw_console(&mut self, ctx: &egui::Context) -> Option<Action> {
        let mut action = None;
        let screen = ctx.content_rect();
        let height = screen.height() * 0.4;
        egui::Area::new(egui::Id::unique("lgduel console"))
            .anchor(egui::Align2::LEFT_TOP, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_gray(27))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        ui.set_width(screen.width() - 16.0);
                        ui.set_height(height);
                        egui::ScrollArea::vertical()
                            .stick_to_bottom(true)
                            .max_height(height - 40.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for l in &self.console.lines {
                                    ui.monospace(l);
                                }
                            });
                        // The ` that toggled the console arrives as text too.
                        self.console.input.retain(|c| c != '`' && c != '~');
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.console.input)
                                .hint_text("help")
                                .desired_width(f32::INFINITY),
                        );
                        edit.request_focus();
                        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let line = std::mem::take(&mut self.console.input);
                            if !line.trim().is_empty() {
                                action = Some(Action::ConsoleLine(line));
                            }
                        }
                    });
            });
        action
    }
}

/// A fixed-width window in the middle of the screen.
fn centered(ctx: &egui::Context, title: &str, width: f32, add: impl FnOnce(&mut egui::Ui)) {
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size(egui::vec2(width, 0.0))
        .show(ctx, add);
}

fn practice_settings(ui: &mut egui::Ui, s: &mut Settings) {
    row(ui, "Map", |ui| {
        combo(ui, "practice_map", &mut s.practice.map, &MAPS);
    });
    row(ui, "Bot", |ui| {
        ui.checkbox(&mut s.practice.bot, "");
    });
    row(ui, "Movement", |ui| {
        combo(ui, "pattern", &mut s.practice.pattern, &PATTERNS);
    });
    row(ui, "Aim", |ui| {
        combo(ui, "skill", &mut s.practice.skill, &SKILLS);
    });
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([170.0, 20.0], egui::Label::new(label));
        add(ui);
    });
}

fn mouse_tab(ui: &mut egui::Ui, s: &mut Settings) {
    row(ui, "cm / 360", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.mouse.cm_per_360)
                .range(1.0..=300.0)
                .speed(0.1)
                .max_decimals(1),
        );
    });
    row(ui, "Mouse DPI", |ui| {
        ui.add(
            egui::DragValue::new(&mut s.mouse.dpi)
                .range(100.0..=32000.0)
                .speed(10.0)
                .max_decimals(0),
        );
    });
    row(ui, "Invert Y", |ui| {
        ui.checkbox(&mut s.mouse.invert_pitch, "");
    });
    ui.label(format!(
        "= {:.4} degrees per count",
        s.mouse.degrees_per_count()
    ));
}

fn video_tab(ui: &mut egui::Ui, s: &mut Settings) {
    row(ui, "Field of view", |ui| {
        ui.add(egui::Slider::new(&mut s.video.fov, 10.0..=130.0).step_by(1.0));
    });
    row(ui, "Fullscreen", |ui| {
        ui.checkbox(&mut s.video.fullscreen, "(F11 / Alt+Enter)");
    });
    row(ui, "Window size", |ui| {
        let cur = s.video.size;
        egui::ComboBox::from_id_salt("size")
            .selected_text(format!("{}x{}", cur.0, cur.1))
            .show_ui(ui, |ui| {
                for r in RESOLUTIONS {
                    ui.selectable_value(&mut s.video.size, r, format!("{}x{}", r.0, r.1));
                }
            });
    });
    row(ui, "Vsync", |ui| {
        ui.checkbox(&mut s.video.vsync, "");
    });
    row(ui, "FPS cap", |ui| {
        let mut on = s.video.fps_cap.is_some();
        ui.checkbox(&mut on, "");
        if on {
            let mut cap = s.video.fps_cap.unwrap_or(240);
            ui.add(egui::DragValue::new(&mut cap).range(30..=2000).speed(5.0));
            s.video.fps_cap = Some(cap);
        } else {
            s.video.fps_cap = None;
        }
    });
}

fn color_row(ui: &mut egui::Ui, label: &str, c: &mut [f32; 3]) {
    row(ui, label, |ui| {
        ui.color_edit_button_rgb(c);
    });
}

fn visuals_tab(ui: &mut egui::Ui, s: &mut Settings, themes: &[String]) {
    ui.heading("Map");
    row(ui, "Theme", |ui| {
        let themes = if themes.is_empty() {
            crate::theme::BUILTIN
                .iter()
                .map(|t| t.to_string())
                .collect()
        } else {
            themes.to_vec()
        };
        egui::ComboBox::from_id_salt("theme")
            .selected_text(s.video.theme.clone())
            .show_ui(ui, |ui| {
                for t in &themes {
                    ui.selectable_value(&mut s.video.theme, t.clone(), t);
                }
            });
    });
    ui.heading("Beam");
    row(ui, "Style", |ui| {
        egui::ComboBox::from_id_salt("beam")
            .selected_text(beam_name(s.effects.beam_style))
            .show_ui(ui, |ui| {
                for b in [
                    BeamStyle::None,
                    BeamStyle::Thin,
                    BeamStyle::Thick,
                    BeamStyle::Zigzag,
                ] {
                    ui.selectable_value(&mut s.effects.beam_style, b, beam_name(b));
                }
            });
    });
    color_row(ui, "Colour", &mut s.effects.beam_color);
    row(ui, "Impact sparks", |ui| {
        ui.checkbox(&mut s.effects.sparks, "");
        ui.add_enabled(
            s.effects.sparks,
            egui::Slider::new(&mut s.effects.spark_size, 0.5..=16.0).text("size"),
        );
    });
    ui.heading("Enemies");
    row(ui, "Model", |ui| {
        use crate::models::ModelKind;
        for (k, name) in [
            (ModelKind::Classic, "Classic"),
            (ModelKind::Humanoid, "Humanoid"),
        ] {
            ui.selectable_value(&mut s.models.enemy, k, name);
        }
    });
    row(ui, "Forced colours", |ui| {
        ui.checkbox(
            &mut s.models.forced_colors,
            "solid colours, easiest to see (recommended)",
        );
    });
    if s.models.forced_colors {
        color_row(ui, "Head", &mut s.enemy_colors.head);
        color_row(ui, "Body", &mut s.enemy_colors.upper);
        color_row(ui, "Legs", &mut s.enemy_colors.lower);
    } else {
        ui.label("Skin colours (shown instead of the forced colours):");
        color_row(ui, "Head", &mut s.models.skin.head);
        color_row(ui, "Torso", &mut s.models.skin.torso);
        color_row(ui, "Arms", &mut s.models.skin.arms);
        color_row(ui, "Legs", &mut s.models.skin.legs);
    }
    ui.heading("Crosshair");
    let c = &mut s.hud.crosshair;
    row(ui, "Length", |ui| {
        ui.add(egui::Slider::new(&mut c.size, 0.0..=40.0).step_by(1.0));
    });
    row(ui, "Gap", |ui| {
        ui.add(egui::Slider::new(&mut c.gap, 0.0..=30.0).step_by(1.0));
    });
    row(ui, "Thickness", |ui| {
        ui.add(egui::Slider::new(&mut c.thickness, 1.0..=8.0).step_by(1.0));
    });
    row(ui, "Centre dot", |ui| {
        ui.checkbox(&mut c.dot, "");
    });
    color_row(ui, "Colour", &mut c.color);
    ui.heading("HUD");
    row(ui, "Damage numbers", |ui| {
        ui.checkbox(&mut s.hud.damage_numbers, "");
    });
    row(ui, "Net-graph (F3)", |ui| {
        ui.checkbox(&mut s.hud.net_graph, "");
    });
    row(ui, "Input display", |ui| {
        ui.checkbox(&mut s.hud.input_display, "W A S D, jump, crouch, fire");
    });
    row(ui, "Gun model", |ui| {
        use crate::effects::GunModel;
        for (g, name) in [
            (GunModel::None, "None"),
            (GunModel::Small, "Small"),
            (GunModel::Normal, "Normal"),
        ] {
            ui.selectable_value(&mut s.effects.gun, g, name);
        }
    });
}

/// Returns true when "Test hit sound" was pressed.
fn audio_tab(ui: &mut egui::Ui, s: &mut Settings) -> bool {
    let mut test = false;
    row(ui, "Volume", |ui| {
        ui.add(egui::Slider::new(&mut s.audio.volume, 0.0..=1.0));
    });
    if ui.button("Test hit sound").clicked() {
        test = true;
    }
    test
}

fn player_tab(ui: &mut egui::Ui, s: &mut Settings) {
    row(ui, "Name", |ui| {
        ui.add(egui::TextEdit::singleline(&mut s.name).char_limit(lgduel_net::protocol::MAX_NAME));
    });
    ui.heading("Practice bot");
    practice_settings(ui, s);
    ui.label("Practice bot changes apply to the next practice session.");
}

fn combo(ui: &mut egui::Ui, id: &str, value: &mut String, options: &[&str]) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.clone())
        .show_ui(ui, |ui| {
            for o in options {
                ui.selectable_value(value, o.to_string(), *o);
            }
        });
}

fn beam_name(b: BeamStyle) -> &'static str {
    match b {
        BeamStyle::None => "none",
        BeamStyle::Thin => "thin",
        BeamStyle::Thick => "thick",
        BeamStyle::Zigzag => "zigzag",
    }
}
