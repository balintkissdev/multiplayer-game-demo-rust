use std::{net::IpAddr, sync::Arc};

use egui::{
    Align2, Button, CentralPanel, Color32, Frame, Grid, Rounding, Shadow, TextEdit, Visuals, Window,
};
use egui_glow::EguiGlow;
use winit::{event::WindowEvent, event_loop::ActiveEventLoop};

use crate::{fsm, globals};

pub struct Gui {
    egui_glow: EguiGlow,
    log_messages: String,
    server_hostname: String,
    server_port: String,
    status_text: String,
    status_color: Color32,
}

impl Gui {
    pub fn new(event_loop: &ActiveEventLoop, gl: Arc<glow::Context>) -> Self {
        let egui_glow = EguiGlow::new(&event_loop, gl, None, None, true);

        egui_glow.egui_ctx.style_mut(|style| {
            style.visuals = Visuals::light();
            style.visuals.window_shadow = Shadow::NONE;
            style.visuals.window_rounding = Rounding::ZERO;
        });

        Self {
            egui_glow,
            log_messages: String::new(),
            server_hostname: String::from(globals::LOCALHOST),
            server_port: globals::DEFAULT_PORT.to_string(),
            status_text: String::from("Ready."),
            status_color: Color32::BLACK,
        }
    }

    pub fn handle_events(&mut self, window: &winit::window::Window, event: &WindowEvent) {
        let _ = self.egui_glow.on_window_event(&window, &event);
    }

    pub fn prepare_frame(
        &mut self,
        window: &winit::window::Window,
        state_machine: &mut fsm::StateMachine,
    ) {
        self.egui_glow
            .run(&window, |ctx| match state_machine.peek() {
                Some(fsm::State::Menu) | Some(fsm::State::Connecting { .. }) => show_menu(
                    ctx,
                    state_machine,
                    &mut self.server_hostname,
                    &mut self.server_port,
                    &mut self.status_text,
                    &mut self.status_color,
                ),
                Some(fsm::State::Playing) => show_log(ctx, &self.log_messages),
                Some(fsm::State::Disconnected) => show_disconnected_dialog(
                    ctx,
                    state_machine,
                    &mut self.log_messages,
                    &mut self.status_text,
                    &mut self.status_color,
                ),
                Some(fsm::State::QuitDialog) => show_quit_dialog(ctx, state_machine),
                _ => {}
            });
    }

    pub fn draw(&mut self, window: &winit::window::Window) {
        self.egui_glow.paint(&window);
    }

    pub fn log(&mut self, msg: String) {
        self.log_messages += &format!("{msg}\n");
    }

    pub fn set_error_status(&mut self, msg: String) {
        self.status_color = Color32::RED;
        self.status_text = msg;
    }
}

fn show_menu(
    ctx: &egui::Context,
    state_machine: &mut fsm::StateMachine,
    server_hostname: &mut String,
    server_port: &mut String,
    status_text: &mut String,
    status_color: &mut Color32,
) {
    Window::new("join_server_menu")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size([300.0, 100.0])
        .show(ctx, |ui| {
            Grid::new("join_server_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Server address:");
                    ui.add(TextEdit::singleline(server_hostname).desired_width(150.0));
                    ui.end_row();

                    ui.label("Port:");
                    ui.add(TextEdit::singleline(server_port).desired_width(150.0));
                    ui.end_row();

                    let connect_buttons_enabled =
                        !matches!(state_machine.peek(), Some(fsm::State::Connecting { .. }));
                    let create_button =
                        ui.add_enabled(connect_buttons_enabled, Button::new("Create server"));
                    let join_button =
                        ui.add_enabled(connect_buttons_enabled, Button::new("Join server"));
                    if create_button.clicked() {
                        match verify_address_format(server_hostname, server_port) {
                            Ok(_) => {
                                *status_text = String::from("Connecting...");
                                *status_color = Color32::BLACK;
                                state_machine.push(fsm::State::Connecting {
                                    server_address: format!("{server_hostname}:{server_port}"),
                                    session_mode: fsm::SessionMode::CreateServer,
                                });
                            }
                            Err(address_parse_err) => {
                                *status_text = address_parse_err;
                                *status_color = Color32::RED;
                            }
                        }
                    }
                    if join_button.clicked() {
                        match verify_address_format(server_hostname, server_port) {
                            Ok(_) => {
                                *status_text = String::from("Connecting...");
                                *status_color = Color32::BLACK;
                                state_machine.push(fsm::State::Connecting {
                                    server_address: format!("{server_hostname}:{server_port}"),
                                    session_mode: fsm::SessionMode::ConnectAsClientOnly,
                                });
                            }
                            Err(address_parse_err) => {
                                *status_text = address_parse_err;
                                *status_color = Color32::RED;
                            }
                        }
                    }
                    ui.colored_label(*status_color, status_text);
                    ui.end_row();

                    if ui.button("Quit").clicked() {
                        state_machine.push(fsm::State::QuitDialog);
                    }
                    ui.end_row();
                });
        });
}

fn show_log(ctx: &egui::Context, log_messages: &String) {
    // Set window transparent just for this widget
    let style = (*ctx.style()).clone();
    ctx.style_mut(|style| {
        style.visuals.window_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 32);
    });

    Window::new("log")
        .title_bar(false)
        .anchor(Align2::LEFT_TOP, egui::Vec2::ZERO)
        .fixed_size([200.0, 80.0])
        .show(&ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.label(log_messages);
                });
        });

    // Reset style back for other dialog widgets
    ctx.set_style(style);
}

fn show_disconnected_dialog(
    ctx: &egui::Context,
    state_machine: &mut fsm::StateMachine,
    log_messages: &mut String,
    status_text: &mut String,
    status_color: &mut Color32,
) {
    CentralPanel::default()
        .frame(Frame::none().fill(Color32::from_black_alpha(192)))
        .show(ctx, |_| {});

    Window::new("disconnected_dialog")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size([300.0, 100.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label("Connection to server was lost.");
                if ui.button("Ok").clicked() {
                    state_machine.change(fsm::State::Menu);
                    log_messages.clear();
                    *status_text = String::from("Ready.");
                    *status_color = Color32::BLACK;
                }
            });
        });
}

fn show_quit_dialog(ctx: &egui::Context, state_machine: &mut fsm::StateMachine) {
    CentralPanel::default()
        .frame(Frame::none().fill(Color32::from_black_alpha(192)))
        .show(ctx, |_| {});

    Window::new("quit_dialog")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .fixed_size([300.0, 100.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label("Are you sure would you like to quit?");
            });

            // HACK: Manual positioning of buttons because centering widgets
            // in horizontal row was not possible.
            let window_rect = ui.max_rect();
            let button_height = 30.0;
            let button_width = 80.0;
            let spacing = 10.0;
            let total_width = button_width * 2.0 + spacing;

            let start_x = window_rect.center().x - total_width / 2.0;
            let start_y = window_rect.max.y - button_height - 10.0;

            let yes_button_rect = egui::Rect::from_min_size(
                egui::pos2(start_x, start_y),
                egui::vec2(button_width, button_height),
            );
            let no_button_rect = egui::Rect::from_min_size(
                egui::pos2(start_x + button_width + spacing, start_y),
                egui::vec2(button_width, button_height),
            );

            if ui.put(yes_button_rect, egui::Button::new("Yes")).clicked() {
                state_machine.change(fsm::State::Quit);
            }
            if ui.put(no_button_rect, egui::Button::new("No")).clicked() {
                state_machine.pop();
            }
        });
}

fn verify_address_format(address: &str, port: &str) -> Result<(), String> {
    match address.parse::<IpAddr>() {
        Ok(_) => {}
        Err(_) => return Err("Error: Invalid IP address format.".to_string()),
    }

    match port.parse::<u16>() {
        Ok(_) => {}
        Err(_) => {
            return Err("Error: Invalid port number. Must be between 0 and 65535.".to_string())
        }
    }

    Ok(())
}
