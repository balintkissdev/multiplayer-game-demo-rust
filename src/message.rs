use std::sync::atomic::{AtomicBool, Ordering};

use cgmath::{Vector2, Vector3};

use crate::{Player, PlayerID};

#[derive(PartialEq)]
pub enum Message {
    Ping,
    Handshake,
    Ack(PlayerID, Vector3<f32>),
    Leave(PlayerID),
    Replicate(Player),
    Position(PlayerID, Vector2<f32>),
}

const PING: &str = "PING";
const HANDSHAKE: &str = "HANDSHAKE";
const ACK: &str = "ACK";
const LEAVE: &str = "LEAVE";
const PUSH: &str = "PUSH";
const POS: &str = "POS";

impl Message {
    pub fn serialize(&self) -> String {
        match self {
            Message::Ping | Message::Handshake => self.name().to_string(),
            Message::Ack(player_id, color) => {
                format!("{}:{}:{}", self.name(), player_id, serialize_color(&color))
            }
            Message::Leave(player_id) => {
                format!("{}:{}", self.name(), player_id)
            }
            Message::Replicate(player_state) => format!(
                "{}:{}:{},{},{}",
                self.name(),
                player_state.id,
                player_state.pos.x as i32,
                player_state.pos.y as i32,
                serialize_color(&player_state.color),
            ),
            Message::Position(player_id, pos) => format!(
                "{}:{}:{},{}",
                self.name(),
                player_id,
                pos.x as i32,
                pos.y as i32
            ),
        }
    }

    pub fn deserialize(msg: &str) -> Result<Message, std::io::Error> {
        let parts: Vec<&str> = msg.split(':').collect();
        match parts.get(0).map(|s| *s) {
            Some(PING) => Ok(Message::Ping),
            Some(HANDSHAKE) => Ok(Message::Handshake),
            Some(ACK) if parts.len() == 3 => {
                let player_id = parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid PlayerID")
                })?;
                let color = deserialize_color(parts[2])
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Message::Ack(player_id, color))
            }
            Some(LEAVE) if parts.len() == 2 => {
                let player_id = parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid PlayerID")
                })?;
                Ok(Message::Leave(player_id))
            }
            Some(PUSH) if parts.len() == 3 => {
                let player_id = parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid PlayerID")
                })?;
                let data_parts: Vec<&str> = parts[2].split(',').collect();
                if data_parts.len() != 3 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid format",
                    ));
                }
                let x = data_parts[0].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid x coordinate")
                })?;
                let y = data_parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid y coordinate")
                })?;
                let color = deserialize_color(data_parts[2])
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                Ok(Message::Replicate(Player {
                    id: player_id,
                    pos: Vector2::new(x, y),
                    velocity: Vector2::new(0.0, 0.0),
                    color,
                }))
            }
            Some(POS) if parts.len() == 3 => {
                let player_id = parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid PlayerID")
                })?;
                let pos_parts: Vec<&str> = parts[2].split(',').collect();
                if pos_parts.len() != 2 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Invalid position format",
                    ));
                }
                let x = pos_parts[0].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid x coordinate")
                })?;
                let y = pos_parts[1].parse().map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid y coordinate")
                })?;
                Ok(Message::Position(player_id, Vector2::new(x, y)))
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unknown or invalid message format",
            )),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Message::Ping => PING,
            Message::Handshake => HANDSHAKE,
            Message::Ack(_, _) => ACK,
            Message::Leave(_) => LEAVE,
            Message::Replicate(_) => PUSH,
            Message::Position(_, _) => POS,
        }
    }
}

fn serialize_color(color: &Vector3<f32>) -> String {
    let r = (color[0] * 255.0).round() as u8;
    let g = (color[1] * 255.0).round() as u8;
    let b = (color[2] * 255.0).round() as u8;

    String::from(format!("#{:02X}{:02X}{:02X}", r, g, b))
}

fn deserialize_color(color_hex: &str) -> Result<Vector3<f32>, String> {
    let color_hex = color_hex.trim_start_matches('#');

    if color_hex.len() != 6 {
        return Err("Invalid hex color format.".to_string());
    }

    let r = u8::from_str_radix(&color_hex[0..2], 16)
        .map_err(|e| format!("Failed to parse red component: {}", e))?;
    let g = u8::from_str_radix(&color_hex[2..4], 16)
        .map_err(|e| format!("Failed to parse green component: {}", e))?;
    let b = u8::from_str_radix(&color_hex[4..6], 16)
        .map_err(|e| format!("Failed to parse blue component: {}", e))?;

    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;

    Ok(Vector3::new(r, g, b))
}

static TRACE_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_trace(enabled: bool) {
    TRACE_ENABLED.store(enabled, Ordering::Relaxed);
}

/// In-house application trace logging functionality.
pub fn trace(s: String) {
    if TRACE_ENABLED.load(Ordering::Relaxed) {
        println!("[TRACE] {s}");
    }
}
