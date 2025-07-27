use axum::{
    extract::{
        State,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{
    sink::SinkExt,
    stream::{SplitStream, StreamExt},
};
use tokio::sync::mpsc;
use tracing::*;

pub mod types;
pub use types::*;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppStateWrapped>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppStateWrapped) {
    let (mut sender, receiver) = socket.split();

    let (tx, mut rx) = mpsc::channel::<ResponseType>(32);

    let peer_id = rand::random_range(2..i64::MAX);
    (*state).lock().await.peers.push(Peer {
        id: peer_id,
        lobby: None,
    });

    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let json = serde_json::to_string_pretty(&msg);
            let msg = match json {
                Ok(json) => json,
                Err(e) => Error::ServerSerialization(e).to_string(),
            };
            let err = sender.send(Message::Text(Utf8Bytes::from(msg))).await;
            if let Err(e) = err {
                error!("{}", e);
            }
        }
    });

    tokio::spawn(async move {
        read(receiver, tx, peer_id, state).await;
    });
}

async fn write(
    mut sender: mpsc::Sender<ResponseType>,
    lobby_id: String,
    mut state: AppStateWrapped,
) {
    // let recv=(*state).lock().await.
}

async fn read(
    mut receiver: SplitStream<WebSocket>,
    mut sender: mpsc::Sender<ResponseType>,
    peer_id: i64,
    mut state: AppStateWrapped,
) {
    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                error!("{}", e);
                continue;
            }
        };

        let msg = match msg {
            Message::Text(utf8_bytes) => utf8_bytes,
            Message::Close(_close_frame) => todo!(),
            _ => {
                warn!("unexpected ws message type received");
                continue;
            }
        };

        let msg = serde_json::from_str::<RequestType>(&msg);

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                error! {"{}", e};
                continue;
            }
        };

        match msg {
            RequestType::Join { lobby_id } => {
                let lobby_id = match lobby_id {
                    Some(lobby_id) => lobby_id,
                    None => create_lobby(true, state.clone()).await,
                };

                if let Err(e) = join_lobby(peer_id, lobby_id, state.clone()).await
                    && let Err(e) = sender.send(ResponseType::Error(e.to_string())).await
                {
                    error!("error sending error{}", e);
                }
            }
            RequestType::Relay { dest_id, message } => {
                
            },
        };
    }
}

async fn create_lobby(mesh: bool, state: AppStateWrapped) -> String {
    let mut state = (*state).lock().await;

    let id = cuid2::CuidConstructor::new().with_length(8).create_id();

    let lobby = Lobby::new(id.clone(), mesh, vec![]);

    state.lobbies.push(lobby);

    id
}



async fn join_lobby(peer_id: i64, lobby_id: String, state: AppStateWrapped) -> Result<(), Error> {
    let mut state = (*state).lock().await;

    // Find indices first instead of mutable references
    let lobby_index = state
        .lobbies
        .iter()
        .position(|e| e.id == lobby_id)
        .ok_or(Error::LobbyDoesNotExist)?;

    let peer_index = state
        .peers
        .iter()
        .position(|e| e.id == peer_id)
        .ok_or(Error::PeerDoesNotExist)?;

    // Now we can get mutable references one at a time
    let lobby = &mut state.lobbies[lobby_index];
    lobby.peers.push(peer_id);

    lobby
        .channel
        .send(ResponseType::PeerConnect { id: peer_id })
        .map_err(Error::ChannelSendError)?;

    lobby
        .channel
        .send(ResponseType::ID {
            id: peer_id,
            lobby_id: lobby_id.clone(),
            mesh: lobby.mesh,
        })
        .map_err(Error::ChannelSendError)?;

    let peer = &mut state.peers[peer_index];
    peer.lobby = Some(lobby_id.clone());
    Ok(())
}
