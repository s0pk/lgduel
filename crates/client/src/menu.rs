//! In-game menus drawn with egui.

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

/// What the player chose this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Resume,
    Disconnect,
    Quit,
}

pub const TITLE: &str = "LGDuel";

/// Draw the current menu screen; returns an action if a button was pressed.
pub fn draw(ctx: &egui::Context, screen: &mut Screen, online: bool) -> Option<Action> {
    let mut action = None;
    match *screen {
        Screen::Closed => {}
        Screen::Pause => {
            egui::Window::new(TITLE)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .fixed_size(egui::vec2(240.0, 0.0))
                .show(ctx, |ui| {
                    ui.vertical_centered_justified(|ui| {
                        if ui.button("Resume").clicked() {
                            action = Some(Action::Resume);
                        }
                        if ui.button("Settings").clicked() {
                            *screen = Screen::Settings;
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
            egui::Window::new("Settings")
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_size(egui::vec2(480.0, 360.0))
                .show(ctx, |ui| {
                    ui.label("Settings are coming in the next update.");
                    if ui.button("Back").clicked() {
                        *screen = Screen::Pause;
                    }
                });
        }
    }
    if action == Some(Action::Resume) {
        *screen = Screen::Closed;
    }
    action
}
