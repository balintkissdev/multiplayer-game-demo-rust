use std::{
    collections::{HashMap, HashSet},
    error::Error,
    time::Duration,
};

use cgmath::{InnerSpace, Vector2};
use tokio::task::JoinHandle;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, KeyCode, NamedKey, PhysicalKey},
    platform::pump_events::EventLoopExtPumpEvents,
    window::Window,
};

use crate::{
    client::ClientSessionResult,
    fsm, globals,
    gui::Gui,
    message::{self, Message},
    server, ClientSession, Player, PlayerID, Renderer,
};

pub fn run_app(rt: &tokio::runtime::Runtime) -> Result<(), Box<dyn Error>> {
    let mut app = App::new(&rt)?;
    let mut event_loop = EventLoop::new()?;
    app.run(&mut event_loop);

    Ok(())
}

#[derive(Eq, Hash, PartialEq)]
enum InputEvent {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
}

type ConnectionTaskHandle = JoinHandle<ClientSessionResult>;
type RemotePlayers = HashMap<PlayerID, Player>; // Access by ID because of position updates

struct App<'a> {
    rt: &'a tokio::runtime::Runtime,
    window: Option<Window>,
    renderer: Option<Renderer>,
    gui: Option<Gui>,
    client_session: Option<ClientSession>,
    connection_task: Option<ConnectionTaskHandle>,
    input_state: HashSet<InputEvent>,
    local_player: Player,
    camera_pos: Vector2<f32>,
    remote_players: RemotePlayers,
    state_machine: fsm::StateMachine,
}

impl<'a> App<'a> {
    fn new(rt: &'a tokio::runtime::Runtime) -> Result<App, Box<dyn Error>> {
        let mut state_machine = fsm::StateMachine::new();
        state_machine.push(fsm::State::Menu);
        Ok(Self {
            rt,
            window: None,
            renderer: None,
            gui: None,
            client_session: None,
            connection_task: None,
            input_state: HashSet::with_capacity(4),
            local_player: Player::default(),
            camera_pos: Vector2::new(0.0, 0.0),
            remote_players: HashMap::new(),
            state_machine,
        })
    }

    fn run(&mut self, event_loop: &mut EventLoop<()>) {
        let mut previous_time = std::time::Instant::now();
        let mut lag: f32 = 0.0;
        loop {
            let current_time = std::time::Instant::now();
            let elapsed_time = (current_time - previous_time).as_secs_f32();
            previous_time = current_time;
            lag += elapsed_time;

            let _ = event_loop.pump_app_events(Some(Duration::ZERO), self);
            if matches!(self.state_machine.peek().unwrap(), fsm::State::Quit) {
                break;
            }
            if self.client_session.is_some() {
                self.process_server_response();
            }

            while lag >= globals::FIXED_UPDATE_TIMESTEP_SEC {
                self.update();
                lag -= globals::FIXED_UPDATE_TIMESTEP_SEC;
            }

            self.window.as_ref().unwrap().request_redraw();
        }

        if self.client_session.is_some() {
            self.client_session
                .as_ref()
                .unwrap()
                .leave_server(self.local_player.id);
        }
    }

    fn process_server_response(&mut self) {
        while let Ok(msg) = self
            .client_session
            .as_mut()
            .unwrap()
            .receive_server_response()
        {
            message::trace(format!("Received: {}", msg));
            match Message::deserialize(&msg) {
                Ok(Message::Replicate(new_player)) => {
                    if let Some(player) = self.remote_players.get_mut(&new_player.id) {
                        // Update existing player based on server's simulation
                        player.pos = new_player.pos;
                    } else {
                        // On-demand remote player creation because replication does not
                        // fit into the handshake ACK message.
                        self.remote_players.insert(new_player.id, new_player);
                        self.gui
                            .as_mut()
                            .unwrap()
                            .log(format!("Player {} has joined the server", new_player.id));
                    }
                }
                Ok(Message::Leave(id)) => {
                    self.remote_players.remove(&id);
                    self.gui
                        .as_mut()
                        .unwrap()
                        .log(format!("Player {} has left the server", id));
                }
                _ => (),
            }
        }
    }

    fn update(&mut self) {
        match self.state_machine.peek_mut() {
            Some(fsm::State::Connecting {
                server_address,
                session_mode,
            }) => {
                // Mechanism to fire connection task once and avoid accidentally spawning
                // infinite number of tasks
                match self.connection_task.as_ref() {
                    Some(task) if task.is_finished() => {
                        if let Some(finished_task) = self.connection_task.take() {
                            let gui = self.gui.as_mut().unwrap();
                            match self.rt.block_on(finished_task) {
                                Ok(result) => match result {
                                    Ok(client_session) => {
                                        self.local_player =
                                            client_session.get_session_player_data();
                                        let window = self.window.as_mut().unwrap();
                                        window.set_title(&format!(
                                            "{} - Player {}",
                                            window.title(),
                                            self.local_player.id
                                        ));
                                        self.client_session = Some(client_session);
                                        self.state_machine.change(fsm::State::Playing);
                                        gui.log(format!(
                                            "Welcome Player {}!",
                                            self.local_player.id
                                        ));
                                    }
                                    Err(connection_err) => {
                                        gui.set_error_status(connection_err.to_string());
                                        self.state_machine.change(fsm::State::Menu);
                                    }
                                },
                                Err(join_err) => {
                                    gui.set_error_status(format!(
                                        "Connection task has aborted: {join_err}"
                                    ));
                                    self.state_machine.change(fsm::State::Menu);
                                }
                            }
                        }
                    }
                    Some(_) => (), // Task is still running, nothing to do
                    None => {
                        // Fire task if not exists
                        let server_address = server_address.clone();
                        let session_mode = session_mode.clone();
                        self.connection_task = Some(self.rt.spawn(async move {
                            if matches!(session_mode, fsm::SessionMode::CreateServer) {
                                let parts: Vec<&str> = server_address.split(':').collect();
                                let port: u16 = parts[1].parse().unwrap();
                                server::start_server(port).await?;
                            }

                            ClientSession::new(server_address).await
                        }));
                    }
                }
            }
            Some(fsm::State::Playing) => {
                let base_speed = 10.0;
                let mut direction = cgmath::vec2(0.0, 0.0);

                if self.input_state.contains(&InputEvent::MoveUp) {
                    direction.y -= 1.0;
                }
                if self.input_state.contains(&InputEvent::MoveDown) {
                    direction.y += 1.0;
                }
                if self.input_state.contains(&InputEvent::MoveLeft) {
                    direction.x -= 1.0;
                }
                if self.input_state.contains(&InputEvent::MoveRight) {
                    direction.x += 1.0;
                }

                // Normalize for consistent movement speed between diagonal and straight directions
                if direction != cgmath::vec2(0.0, 0.0) {
                    direction = direction.normalize();
                }

                self.local_player.velocity = direction * base_speed;
                self.local_player.pos += self.local_player.velocity;
                globals::clamp_player_to_bounds(&mut self.local_player);

                self.move_camera();

                if self.local_player.velocity != cgmath::vec2(0.0, 0.0) {
                    self.client_session
                        .as_ref()
                        .unwrap()
                        .send_pos(&self.local_player);
                }

                if !self.client_session.as_ref().unwrap().is_server_alive() {
                    eprintln!("Connection to server was lost");
                    self.client_session = None;
                    self.window
                        .as_mut()
                        .unwrap()
                        .set_title(globals::WINDOW_TITLE);
                    self.input_state.clear(); // Avoid keys being stuck
                    self.remote_players.clear();
                    self.state_machine.change(fsm::State::Disconnected);
                }
            }
            _ => (),
        }
    }

    fn move_camera(&mut self) {
        let half_width = globals::WINDOW_SIZE.0 as f32 / 2.0;
        let half_height = globals::WINDOW_SIZE.1 as f32 / 2.0;

        // Calculate the camera's allowed range
        let min_camera_x = globals::WORLD_BOUNDS.min_x + half_width;
        let max_camera_x = globals::WORLD_BOUNDS.max_x - half_width;
        let min_camera_y = globals::WORLD_BOUNDS.min_y + half_height;
        let max_camera_y = globals::WORLD_BOUNDS.max_y - half_height;

        // Update camera position, clamping to the allowed range
        self.camera_pos.x = self.local_player.pos.x.clamp(min_camera_x, max_camera_x);
        self.camera_pos.y = self.local_player.pos.y.clamp(min_camera_y, max_camera_y);
    }
}

impl ApplicationHandler for App<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let (window, renderer, gui) = Renderer::create_graphics(&event_loop);

        self.window = Some(window);
        self.renderer = Some(renderer);
        self.gui = Some(gui);
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let window = self.window.as_ref().unwrap();
        let gui = self.gui.as_mut().unwrap();

        match event {
            WindowEvent::CloseRequested => self.state_machine.change(fsm::State::Quit),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(physical_key),
                        ref logical_key,
                        repeat: false,
                        state,
                        ..
                    },
                is_synthetic: false,
                ..
            } => {
                if matches!(logical_key, Key::Named(NamedKey::Escape)) &&
                // Negation is an additional guard to avoid accidentally pushing duplicate states when someone holds down Esc key for too long
                !matches!(self.state_machine.peek(), Some(fsm::State::QuitDialog))
                {
                    self.state_machine.push(fsm::State::QuitDialog);
                }

                if matches!(self.state_machine.peek(), Some(fsm::State::Playing)) {
                    let input_event = match physical_key {
                        KeyCode::ArrowUp | KeyCode::KeyW => Some(InputEvent::MoveUp),
                        KeyCode::ArrowDown | KeyCode::KeyS => Some(InputEvent::MoveDown),
                        KeyCode::ArrowLeft | KeyCode::KeyA => Some(InputEvent::MoveLeft),
                        KeyCode::ArrowRight | KeyCode::KeyD => Some(InputEvent::MoveRight),
                        _ => None,
                    };
                    if input_event.is_some() {
                        match state {
                            ElementState::Pressed => self.input_state.insert(input_event.unwrap()),
                            ElementState::Released => {
                                self.input_state.remove(&input_event.unwrap())
                            }
                        };
                    }
                }
            }
            WindowEvent::Focused(false) => {
                // Avoid stuck keys when window loses focus
                self.input_state.clear();
            }
            WindowEvent::RedrawRequested => {
                let renderer = self.renderer.as_ref().unwrap();

                gui.prepare_frame(&window, &mut self.state_machine);
                renderer.draw(
                    &self.camera_pos,
                    &self.local_player,
                    &self.remote_players,
                    self.state_machine.peek(),
                );
                gui.draw(&window);
                renderer.swap_buffers();
            }
            _ => (),
        }

        gui.handle_events(&window, &event);
    }
}
