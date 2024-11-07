pub mod app;
pub mod client;
pub use client::ClientSession;
pub mod fsm;
pub use fsm::StateMachine;
pub mod gui;
pub mod message;
mod renderer;
pub use renderer::Renderer;
pub mod server;

use cgmath::{Vector2, Vector3};

type PlayerID = u64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub id: PlayerID,
    pub pos: Vector2<f32>,
    pub velocity: Vector2<f32>,
    pub color: Vector3<f32>,
}

impl Player {
    pub fn new(id: PlayerID, color: Vector3<f32>) -> Self {
        let mut player = Player::default();
        player.id = id;
        player.color = color;
        player
    }
}

impl Default for Player {
    fn default() -> Self {
        Self {
            id: 0,
            pos: Vector2::new(0.0, 0.0),
            velocity: Vector2::new(0.0, 0.0),
            color: Vector3::new(0.0, 0.0, 0.0),
        }
    }
}

pub struct WorldBounds {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
}

pub mod globals {
    use crate::{Player, WorldBounds};

    pub const LOCALHOST: &str = "127.0.0.1";
    pub const DEFAULT_PORT: u16 = 8080;
    pub const PING_INTERVAL_MS: std::time::Duration = std::time::Duration::from_millis(20);
    pub const CONNECTION_TIMEOUT_SEC: std::time::Duration = std::time::Duration::from_secs(5);

    pub const WINDOW_SIZE: (u16, u16) = (800, 600);
    pub const WINDOW_TITLE: &str = "Multiplayer game demo by Bálint Kiss";

    /// This is the granularity of how often to update logic and not to be confused
    /// with framerate limiting or 60 frames per second, because the main loop
    /// implementation uses a fixed update, variable framerate timestep algorithm.
    ///
    /// 60 logic updates per second is a common value used in games.
    /// - Higher update rate (120) can lead to smoother gameplay, more precise
    /// control, at the cost of CPU load. Keep mobile devices in mind.
    /// - Lower update rate (30) reduces CPU load, runs game logic less frequently,
    /// but can make game less responsive.
    pub const MAX_LOGIC_UPDATE_PER_SEC: f32 = 60.0;
    pub const FIXED_UPDATE_TIMESTEP_SEC: f32 = 1.0 / MAX_LOGIC_UPDATE_PER_SEC;

    pub const PLAYER_QUAD_SIZE: f32 = 24.0;

    /// World bounds are relative to origin (0,0)
    pub const WORLD_BOUNDS: WorldBounds = WorldBounds {
        min_x: -1200.0,
        min_y: -1200.0,
        max_x: 1200.0,
        max_y: 1200.0,
    };

    pub fn clamp_player_to_bounds(player: &mut Player) {
        player.pos.x = player.pos.x.clamp(
            WORLD_BOUNDS.min_x + (PLAYER_QUAD_SIZE / 2.0),
            WORLD_BOUNDS.max_x - (PLAYER_QUAD_SIZE / 2.0),
        );
        player.pos.y = player.pos.y.clamp(
            WORLD_BOUNDS.min_y + (PLAYER_QUAD_SIZE / 2.0),
            WORLD_BOUNDS.max_y - (PLAYER_QUAD_SIZE / 2.0),
        );
    }
}
