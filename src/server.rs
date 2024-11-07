use cgmath::{Vector2, Vector3};
use rand::Rng;
use std::{
    collections::HashMap,
    error::Error,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, Mutex},
};

use crate::{
    globals,
    message::{self, Message},
    Player, PlayerID,
};

pub type ServerSessionResult = Result<(), Box<dyn Error + Send + Sync>>;

pub async fn start_server(port: u16) -> ServerSessionResult {
    match tokio::time::timeout(globals::CONNECTION_TIMEOUT_SEC, async {
        let addr = format!("0.0.0.0:{port}"); // Make sure to listen on all interfaces
        let server_socket = UdpSocket::bind(&addr).await?;
        let (broadcast_tx, broadcast_rx) = mpsc::unbounded_channel::<BroadcastMessage>();
        let context = Arc::new(ServerContext::new(server_socket, broadcast_tx.clone()));

        tokio::spawn(broadcast_sender(context.clone(), broadcast_rx));
        tokio::spawn(listen_handler(context.clone()));
        println!("Listening on UDP port {port}");

        Ok(()) as ServerSessionResult
    })
    .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "Server creation timed out after {} seconds: {e}",
            globals::CONNECTION_TIMEOUT_SEC.as_secs()
        )
        .into()),
    }
}

type PlayerMap = HashMap<SocketAddr, Player>;

struct BroadcastMessage {
    msg: Vec<u8>,
    excluded_client: Option<SocketAddr>,
}

// Non-blocking channels are used for lock-free message passing from sync main thread to async
// context and between multiple async tasks.
// TODO: Research how to handle backpressure
type ChannelSender = mpsc::UnboundedSender<BroadcastMessage>;
type ChannelReceiver = mpsc::UnboundedReceiver<BroadcastMessage>;

/// Parameter object accessible from multiple async tasks
struct ServerContext {
    server_socket: UdpSocket,
    broadcast_tx: ChannelSender,
    players: Mutex<PlayerMap>,
    /// ID acting as player number, increases on every new player
    /// join
    player_id_counter: AtomicU64,
}

impl ServerContext {
    fn new(server_socket: UdpSocket, broadcast_tx: ChannelSender) -> Self {
        Self {
            server_socket,
            broadcast_tx,
            players: Mutex::new(PlayerMap::new()),
            player_id_counter: AtomicU64::new(1),
        }
    }
}

/// Primary listener loop for incoming client UDP requests, processing each new message in separate task.
async fn listen_handler(context: Arc<ServerContext>) {
    loop {
        let mut buf = [0u8; 32];
        // TODO: Consider non-blocking UDP I/O
        let (len, client) = context.server_socket.recv_from(&mut buf).await.unwrap();
        if 1 < len {
            let request_msg = String::from_utf8_lossy(&buf[..len]).to_string();
            tokio::spawn(process_client_message(context.clone(), client, request_msg));
        }
    }
}

/// Sender loop for broadcasting server UDP responses to all players except the player who owning
/// the broadcast message.
async fn broadcast_sender(context: Arc<ServerContext>, mut broadcast_rx: ChannelReceiver) {
    while let Some(broadcast) = broadcast_rx.recv().await {
        message::trace(format!(
            "Broadcasting: {}",
            String::from_utf8_lossy(&broadcast.msg)
        ));
        let players = context.players.lock().await;
        for (client_addr, _) in players.iter() {
            if Some(*client_addr) != broadcast.excluded_client {
                if let Err(e) = context
                    .server_socket
                    .send_to(&broadcast.msg, client_addr)
                    .await
                {
                    eprintln!("Failed to broadcast: {:?}", e);
                }
            }
        }
    }
}

/// Periodic ping sender that clients can use as healthcheck of server.
async fn ping_sender(context: Arc<ServerContext>) {
    let mut interval = tokio::time::interval(globals::PING_INTERVAL_MS);
    loop {
        interval.tick().await;
        let _ = context.broadcast_tx.send(BroadcastMessage {
            msg: Message::Ping.serialize().into_bytes(),
            excluded_client: None,
        });
    }
}

/// Authoritative game update logic simulation.
///
/// Requires fixed processing, because timing has to be synchronized accross all connected clients.
/// A server simulation loop does not need to play "catch-up" like a local game loop does, because
/// there's no point in sending stale packets.
async fn simulation_handler(context: Arc<ServerContext>) {
    let desired_frame_duration =
        std::time::Duration::from_secs_f32(globals::FIXED_UPDATE_TIMESTEP_SEC);
    let mut interval = tokio::time::interval(desired_frame_duration);

    interval.tick().await; // Skip the first tick (or else there will be bugs)

    loop {
        let current_time = std::time::Instant::now();

        {
            let mut players = context.players.lock().await;
            for (client, player) in players.iter_mut() {
                // Bounds check
                globals::clamp_player_to_bounds(player);

                // Gameplay state replication
                let msg = Message::Replicate(*player).serialize();
                let _ = context.broadcast_tx.send(BroadcastMessage {
                    msg: msg.into_bytes(),
                    excluded_client: Some(*client),
                });
            }
        } // Release the lock as soon as possible

        let elapsed_time = current_time.elapsed();
        if elapsed_time < desired_frame_duration {
            interval.tick().await;
        }
    }
}

async fn process_client_message(context: Arc<ServerContext>, client: SocketAddr, msg: String) {
    message::trace(format!("Received: {msg}"));
    match Message::deserialize(&msg) {
        Ok(Message::Handshake) => {
            accept_client(context, client).await.unwrap();
        }
        Ok(Message::Position(player_id, pos)) => {
            update_position(context, client, player_id, pos)
                .await
                .unwrap();
        }
        Ok(Message::Leave(player_id)) => {
            drop_player(context, client, player_id).await.unwrap();
        }
        _ => (),
    }
}

/// Recieve first time joining client handshake, register as new player and send ACK response
/// with new player info.
///
/// Each new player receives a randomly generated color and the player ID counter is incremented
/// after each new join.
async fn accept_client(
    context: Arc<ServerContext>,
    client: SocketAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut players = context.players.lock().await;

    let ack_msg: String;
    if let Some(existing_player) = players.get(&client) {
        // Getting multiple handshakes from and sending out multiple ACK for the same
        // client is not a problem, that just means that previous ACK was dropped, so the
        // client retried the HANDSHAKE. Server just resends ACK with same player info that
        // was already registered as response to new HANDSHAKE. It is made sure here not to
        // accidentally add the same player multiple times, because that would lead to
        // "Player 3 joined, Player
        // 4 joined, Player 5 joined" bug for each accepted HANDSHAKE from the same client.
        ack_msg = Message::Ack(existing_player.id, existing_player.color).serialize();
    } else {
        // Add new player to server
        let new_player = Player::new(
            context.player_id_counter.fetch_add(1, Ordering::SeqCst),
            generate_color(),
        );
        players.insert(client, new_player);
        println!("Player {} joined the server", new_player.id);

        // First time game startup: start sending out PING messages (to everyone) and start the
        // game simulation itself when the first player has connected
        if players.len() == 1 {
            tokio::spawn(ping_sender(context.clone()));
            tokio::spawn(simulation_handler(context.clone()));
        }

        ack_msg = Message::Ack(new_player.id, new_player.color).serialize();
    }

    // Send ACK
    context
        .server_socket
        .send_to(ack_msg.as_bytes(), client)
        .await?;
    message::trace(format!("Sent: {ack_msg}"));

    Ok(())
}

async fn update_position(
    context: Arc<ServerContext>,
    client: SocketAddr,
    player_id: PlayerID,
    new_pos: Vector2<f32>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    if let Some(player) = context.players.lock().await.get_mut(&client) {
        if player_id != player.id {
            return Ok(());
        }

        player.pos.x = new_pos.x;
        player.pos.y = new_pos.y;
    }

    Ok(())
}

// FIXME: LEAVE packets from can be dropped
async fn drop_player(
    context: Arc<ServerContext>,
    client: SocketAddr,
    player_id: PlayerID,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut players = context.players.lock().await;
    players.remove(&client);

    println!("Player {player_id} left the server");
    context.broadcast_tx.send(BroadcastMessage {
        msg: Message::Leave(player_id).serialize().into_bytes(),
        excluded_client: Some(client),
    })?;

    Ok(())
}

fn generate_color() -> Vector3<f32> {
    let mut rng = rand::thread_rng();
    // Avoid generating white color
    loop {
        let r = rng.gen_range(0.0..=1.0);
        let g = rng.gen_range(0.0..=1.0);
        let b = rng.gen_range(0.0..=1.0);

        if r < 1.0 || g < 1.0 || b < 1.0 {
            return Vector3::new(r, g, b);
        }
    }
}
