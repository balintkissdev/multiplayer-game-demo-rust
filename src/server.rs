use cgmath::{num_traits::ToPrimitive, Vector2, Vector3};
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
    match tokio::time::timeout(globals::CONNECTION_TIMEOUT_DURATION, async {
        let addr = format!("{}:{}", globals::LOCALHOST, port);
        let server_socket = UdpSocket::bind(&addr).await?;
        let (broadcast_tx, broadcast_rx) = mpsc::unbounded_channel::<BroadcastMessage>();
        let context = Arc::new(ServerContext::new(server_socket, broadcast_tx.clone()));

        tokio::spawn(broadcast_handler(context.clone(), broadcast_rx));
        tokio::spawn(listen_handler(context.clone()));
        println!("Listening on {addr}");

        Ok(()) as ServerSessionResult
    })
    .await
    {
        Ok(_) => Ok(()),
        Err(e) => Err(format!(
            "Server creation timed out after {} seconds: {e}",
            globals::CONNECTION_TIMEOUT_DURATION.as_secs()
        )
        .into()),
    }
}

type PlayerMap = HashMap<SocketAddr, Player>;

struct BroadcastMessage {
    msg: Vec<u8>,
    excluded_client: Option<SocketAddr>,
}

type ChannelSender = mpsc::UnboundedSender<BroadcastMessage>;
type ChannelReceiver = mpsc::UnboundedReceiver<BroadcastMessage>;

struct ServerContext {
    server_socket: UdpSocket,
    broadcast_tx: ChannelSender,
    players: Mutex<PlayerMap>,
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

async fn listen_handler(context: Arc<ServerContext>) {
    loop {
        let mut buf = [0u8; 32];
        let (len, client) = context.server_socket.recv_from(&mut buf).await.unwrap();
        if 1 < len {
            let msg = String::from_utf8_lossy(&buf[..len]).to_string();
            tokio::spawn(process_client_message(context.clone(), client, msg));
        }
    }
}

async fn broadcast_handler(context: Arc<ServerContext>, mut broadcast_rx: ChannelReceiver) {
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

async fn ping_handler(context: Arc<ServerContext>) {
    let mut interval = tokio::time::interval(globals::PING_INTERVAL);
    loop {
        interval.tick().await;
        let _ = context.broadcast_tx.send(BroadcastMessage {
            msg: Message::Ping.serialize().into_bytes(),
            excluded_client: None,
        });
    }
}

async fn process_client_message(context: Arc<ServerContext>, client: SocketAddr, msg: String) {
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

async fn accept_client(
    context: Arc<ServerContext>,
    client: SocketAddr,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut players = context.players.lock().await;

    // Add new player to server
    let new_player = Player {
        id: context
            .player_id_counter
            .fetch_add(1, Ordering::SeqCst)
            .to_u64()
            .unwrap(),
        pos: cgmath::vec2(0.0, 0.0),
        velocity: cgmath::vec2(0.0, 0.0),
        color: generate_color(),
    };
    players.insert(client, new_player);
    let msg = Message::Ack(new_player.id, new_player.color).serialize();
    context
        .server_socket
        .send_to(msg.as_bytes(), client)
        .await?;
    message::trace(format!("Sent: {msg}"));

    // Init new player with positions of existing players
    for (existing_client, _) in players.iter() {
        if *existing_client != client {
            let existing_player = players.get(existing_client).unwrap();
            let msg = Message::Replicate(*existing_player).serialize();
            context
                .server_socket
                .send_to(msg.as_bytes(), client)
                .await?;
            message::trace(format!("Sent: {msg}"));
        }
    }

    // Start sending out PING messages when the first player has connected
    if players.len() == 1 {
        tokio::spawn(ping_handler(context.clone()));
    }

    // Notify existing players about new player
    println!("Player {} joined the server", new_player.id);
    context.broadcast_tx.send(BroadcastMessage {
        msg: Message::Replicate(new_player).serialize().into_bytes(),
        excluded_client: Some(client),
    })?;

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
        globals::clamp_player_to_bounds(player);

        context.broadcast_tx.send(BroadcastMessage {
            msg: Message::Position(player.id, player.pos)
                .serialize()
                .into_bytes(),
            excluded_client: Some(client),
        })?;
    }

    Ok(())
}

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
