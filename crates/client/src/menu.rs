//! In-game menus drawn with egui: the Esc menu and the settings screen.
//! Settings edit a `Settings` value in place; the app applies changes live and
//! saves them when the settings screen is left.

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
    /// Settings (opened from the pause menu).
    Settings,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Resume,
    Disconnect,
    Quit,
    /// Play the hit sound once (audio test).
    TestSound,
    /// The settings screen was closed (time to save).
    SettingsClosed,
}

#[derive(Debug, Clone, Default)]
pub struct Menu {
    pub screen: Screen,
    pub tab: Tab,
    /// Settings changed since the screen was opened.
    pub dirty: bool,
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

impl Menu {
    pub fn is_open(&self) -> bool {
        self.screen != Screen::Closed
    }

    /// Draw the current screen; may edit `s` (sets `dirty`) and return an action.
    pub fn draw(&mut self, ctx: &egui::Context, s: &mut Settings, online: bool) -> Option<Action> {
        let mut action = None;
        match self.screen {
            Screen::Closed => {}
            Screen::Pause => {
                egui::Window::new(TITLE)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .fixed_size(egui::vec2(260.0, 0.0))
                    .show(ctx, |ui| {
                        ui.vertical_centered_justified(|ui| {
                            if ui.button("Resume").clicked() {
                                action = Some(Action::Resume);
                            }
                            if ui.button("Settings").clicked() {
                                self.screen = Screen::Settings;
                                self.dirty = false;
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
                                Tab::Visuals => visuals_tab(ui, s),
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
                            self.screen = Screen::Pause;
                            action = Some(Action::SettingsClosed);
                        }
                    });
            }
        }
        if action == Some(Action::Resume) {
            self.screen = Screen::Closed;
        }
        action
    }
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

fn visuals_tab(ui: &mut egui::Ui, s: &mut Settings) {
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
    ui.heading("Enemy colours");
    color_row(ui, "Head", &mut s.enemy_colors.head);
    color_row(ui, "Body", &mut s.enemy_colors.upper);
    color_row(ui, "Legs", &mut s.enemy_colors.lower);
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
    row(ui, "Bot", |ui| {
        ui.checkbox(&mut s.practice.bot, "");
    });
    row(ui, "Movement", |ui| {
        combo(ui, "pattern", &mut s.practice.pattern, &PATTERNS);
    });
    row(ui, "Aim", |ui| {
        combo(ui, "skill", &mut s.practice.skill, &SKILLS);
    });
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
