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
        match tokio::time::timeout(globals::CONNECTION_TIMEOUT_SEC, async {
            let client_socket = UdpSocket::bind("0.0.0.0:0").await?;
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
                globals::CONNECTION_TIMEOUT_SEC.as_secs()
            )
            .into()),
        }
    }

    pub fn get_session_player_data(&self) -> Player {
        self.session_player
    }

    pub fn receive_server_response(&mut self) -> Result<String, TryRecvError> {
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
        self.last_ping.elapsed() < globals::CONNECTION_TIMEOUT_SEC
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
    let handshake_msg = Message::Handshake.serialize();
    // Loop abort happens on timeout in ClientSession::new()
    loop {
        client_socket
            .send_to(handshake_msg.as_bytes(), server_address)
            .await?;
        message::trace(format!("Sent: {handshake_msg}"));

        match receive_with_retry_timeout(client_socket).await {
            Ok(response) => {
                if let Ok(Message::Ack(new_id, new_color)) = Message::deserialize(&response) {
                    message::trace(format!("Handshake result: {response}"));
                    return Ok(Player::new(new_id, new_color));
                }

                message::trace(format!("Invalid handshake response: {response}"));
            }
            _ => continue, // Keep trying, I know you can do it!
        }
    }
}

async fn receive_with_retry_timeout(
    socket: &UdpSocket,
) -> Result<String, Box<dyn Error + Send + Sync>> {
    let retry_timeout = std::time::Duration::from_millis(300);
    let mut buf = [0u8; 32];
    match tokio::time::timeout(retry_timeout, socket.recv_from(&mut buf)).await {
        Ok(result) => {
            let (len, _) = result?;
            Ok(String::from_utf8_lossy(&buf[..len]).to_string())
        }
        Err(_) => {
            message::trace("No response (sender or receiver package lost)".to_string());
            Err("Receive operation timed out".into())
        }
    }
}

async fn listen_handler(socket: Arc<UdpSocket>, listen_tx: ChannelSender) {
    let mut buf = [0u8; 1024];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((len, _)) => {
                if let Ok(msg) = std::str::from_utf8(&buf[..len]) {
                    // Pass message to main thread
                    if listen_tx.send(msg.to_string()).is_err() {
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
