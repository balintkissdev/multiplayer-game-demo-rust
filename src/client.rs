use std::{error::Error, sync::Arc};

use tokio::{
    net::UdpSocket,
    sync::mpsc::{self, error::TryRecvError},
    task::JoinHandle,
};

use crate::{
    globals,
    message::{self, Message},
    Player, PlayerID,
};

type ChannelSender = mpsc::UnboundedSender<String>;
type ChannelReceiver = mpsc::UnboundedReceiver<String>;

pub struct ClientSession {
    listen_rx: ChannelReceiver,
    send_tx: ChannelSender,
    listen_task: JoinHandle<()>,
    send_task: JoinHandle<()>,
    session_player: Player,
    last_ping: std::time::Instant,
}

pub type ClientSessionResult = Result<ClientSession, Box<dyn Error + Send + Sync>>;

impl ClientSession {
    pub async fn new(server_address: String) -> ClientSessionResult {
        match tokio::time::timeout(globals::CONNECTION_TIMEOUT_DURATION, async {
            let client_socket = UdpSocket::bind("127.0.0.1:0").await?;
            let client_socket = Arc::new(client_socket);

            let session_player = join_server(&client_socket, &server_address).await?;

            let (listen_tx, listen_rx) = mpsc::unbounded_channel();
            let (send_tx, send_rx) = mpsc::unbounded_channel();
            let listen_task = tokio::spawn(listen_handler(client_socket.clone(), listen_tx));
            let send_task = tokio::spawn(send_handler(
                client_socket.clone(),
                server_address.clone(),
                send_rx,
            ));

            println!("Connected to server");
            Ok(Self {
                listen_rx,
                send_tx,
                listen_task,
                send_task,
                session_player,
                last_ping: std::time::Instant::now(),
            })
        })
        .await
        {
            Ok(client_session) => client_session,
            Err(_) => Err(format!(
                "Connection timed out after {} seconds.",
                globals::CONNECTION_TIMEOUT_DURATION.as_secs()
            )
            .into()),
        }
    }

    pub fn get_session_player_data(&self) -> Player {
        self.session_player
    }

    pub fn receive_server_resposne(&mut self) -> Result<String, TryRecvError> {
        match self.listen_rx.try_recv() {
            Ok(response) => {
                if let Ok(Message::Ping) = Message::deserialize(&response) {
                    self.last_ping = std::time::Instant::now();
                }
                Ok(response)
            }
            Err(e) => Err(e),
        }
    }

    pub fn send_pos(&self, player: &Player) {
        let _ = self
            .send_tx
            .send(Message::Position(player.id, player.pos).serialize());
    }

    pub fn is_server_alive(&self) -> bool {
        self.last_ping.elapsed() < globals::CONNECTION_TIMEOUT_DURATION
    }

    pub fn leave_server(&self, player_id: PlayerID) {
        let _ = self.send_tx.send(Message::Leave(player_id).serialize());
    }
}

impl Drop for ClientSession {
    fn drop(&mut self) {
        self.listen_task.abort();
        self.send_task.abort();
        self.listen_rx.close();
    }
}

// Joining a server is a synchronized handshake procedure.
async fn join_server(
    client_socket: &UdpSocket,
    server_address: &String,
) -> Result<Player, Box<dyn Error + Send + Sync>> {
    let msg = Message::Handshake.serialize();
    client_socket
        .send_to(msg.as_bytes(), server_address)
        .await?;
    message::trace(format!("Sent: {msg}"));

    let mut ack_buf = [0u8; 32];
    let (len, _) = client_socket.recv_from(&mut ack_buf).await?;
    let response = String::from_utf8_lossy(&ack_buf[..len]).to_string();
    message::trace(format!("Handshake result: {response}"));

    let result = match Message::deserialize(&response) {
        Ok(Message::Ack(new_id, new_color)) => Ok(Player::new(new_id, new_color)),
        Ok(_) => Err("Invalid handshake received".into()),
        Err(e) => Err(format!("Handshake failed: {e}").into()),
    };
    result
}

async fn listen_handler(socket: Arc<UdpSocket>, tx: ChannelSender) {
    let mut buf = [0u8; 1024];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, _)) => {
                if let Ok(msg) = std::str::from_utf8(&buf[..len]) {
                    // Pass message to main thread
                    if tx.send(msg.to_string()).is_err() {
                        break;
                    }
                }
            }
            Err(_) => {
                break;
            }
        }
    }
}

async fn send_handler(socket: Arc<UdpSocket>, server_address: String, mut rx: ChannelReceiver) {
    while let Some(msg) = rx.recv().await {
        let _ = socket.send_to(&msg.as_bytes(), &server_address).await;
        message::trace(format!("Sent: {msg}"));
    }
}
