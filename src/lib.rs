use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{
    sink::SinkExt,
    stream::{SplitStream, StreamExt},
};
use tokio::{
    sync::{broadcast, mpsc},
    time::sleep,
};
use tracing::{field::ValueSet, *};

pub mod types;
pub use types::*;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppStateWrapped>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

#[derive(Debug, Clone)]
enum WSMessagePass {
    Raw(Message),
    Typed(ResponseType),
}

#[tracing::instrument]
async fn handle_socket(socket: WebSocket, state: AppStateWrapped) {
    let (mut sender, receiver) = socket.split();

    let (tx, mut rx) = mpsc::channel::<WSMessagePass>(100);

    let peer_id = rand::random_range(2..i64::MAX);

    info!("new socket connection, id:{peer_id}");

    state.lock().await.peers.insert(
        peer_id,
        Peer {
            id: peer_id,
            lobby: None,
        },
    );

    info!("inserted peer");

    tokio::spawn(async move {
        let span = span!(Level::TRACE, "message passer");
        let _enter = span.enter();
        info!("entered message passer task");
        while let Some(msg) = rx.recv().await {
            let mut close = false;
            let msg = match msg {
                WSMessagePass::Raw(message) => {
                    if let Message::Close(_) = message {
                        info!("closing socket");
                        close = true
                    }
                    message
                }
                WSMessagePass::Typed(response_type) => {
                    let json = serde_json::to_string_pretty(&response_type);
                    let msg = match json {
                        Ok(json) => json,
                        Err(e) => Error::ServerSerialization(e).to_string(),
                    };
                    Message::Text(Utf8Bytes::from(msg))
                }
            };

            let err = sender.send(msg).await;

            if let Err(e) = err {
                error!("{}", e);
            }

            if close {
                if let Err(e) = sender.close().await {
                    error!("error closing socket: {e}");
                }
                break;
            }
        }
        info!("exited message passer task");
    });

    {
        let tx = tx.clone();
        let state = state.clone();

        tokio::spawn(async move {
            read(receiver, tx, peer_id, state).await;
        });
    }

    tokio::spawn(async move {
        write(tx, peer_id, state).await;
    });
}

async fn write(sender: mpsc::Sender<WSMessagePass>, peer_id: i64, state: AppStateWrapped) {
    loop {
        let mut state = state.lock().await;
        let l = if let Some(p) = state.peers.get(&peer_id).cloned()
            && let Some(l_id) = p.lobby
            && let Some(l) = state.lobbies.get_mut(&l_id)
        {
            l
        } else {
            sleep(Duration::from_millis(500)).await;
            continue;
        };

        let mut channel = l.channel.subscribe();

        loop {
            let msg = channel.recv().await;

            let msg = match msg {
                Ok(m) => m,
                Err(broadcast::error::RecvError::Closed) => {
                    error!("channel closed for lobby {}", l.id);
                    sleep(Duration::from_millis(500)).await;
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(lag)) => {
                    warn!("lobby {} lagged {lag} messages", l.id);
                    continue;
                }
            };

            let msg = match msg {
                ResponseType::ID { .. } => continue,
                ResponseType::PeerConnect { id } | ResponseType::PeerDisconnect { id } => {
                    if id != peer_id {
                        msg
                    } else {
                        continue;
                    }
                }
                ResponseType::Relay {
                    sender_id: _,
                    dest_id,
                    message: _,
                } => {
                    if dest_id == peer_id {
                        msg
                    } else {
                        continue;
                    }
                }
                ResponseType::Error { .. } => continue,
            };

            if let Err(e) = sender.send(WSMessagePass::Typed(msg)).await {
                error!("error while sending to message passer thread: {e}");
            }
        }
    }
}

async fn read(
    mut receiver: SplitStream<WebSocket>,
    sender: mpsc::Sender<WSMessagePass>,
    peer_id: i64,
    state: AppStateWrapped,
) {
    while let Some(msg) = receiver.next().await {
        let span = span!(Level::TRACE, "request");
        let _enter = span.enter();

        info!("new ws message:{:#?}", msg);

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                error!("error receiving websocket message: {}", e);
                continue;
            }
        };

        let msg = match msg {
            Message::Text(utf8_bytes) => utf8_bytes,
            Message::Close(_close_frame) => {
                let mut state = state.lock().await;
                if let Some(Some(id)) = state.peers.get(&peer_id).map(|p| &p.lobby).cloned()
                    && let Some(lobby) = state.lobbies.get_mut(&id)
                    && let Err(e) = lobby
                        .channel
                        .send(ResponseType::PeerDisconnect { id: peer_id })
                {
                    error!("error sending disconnect message: {e}")
                }
                if let Err(e) = sender
                    .send(WSMessagePass::Raw(Message::Close(Some(CloseFrame {
                        code: 1000,
                        reason: Utf8Bytes::from_static("closing based on client request"),
                    }))))
                    .await
                {
                    error!("error sending closing message to message passer: {e}")
                }

                break;
            }
            _ => {
                continue;
            }
        };

        info!("text ws message: {msg}");

        let msg = serde_json::from_str::<RequestType>(&msg);

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                error! {"error deserializing request json {}", e};
                continue;
            }
        };

        info!("deserialized msg: {:#?}", msg);

        match msg {
            RequestType::Join { lobby_id } => {
                let lobby_id = match lobby_id {
                    Some(lobby_id) => lobby_id,
                    None => create_lobby(true, state.clone()).await,
                };

                info!("lobby id: {lobby_id}");

                if let Err(e) = join_lobby(peer_id, lobby_id.clone(), state.clone()).await {
                    let e = format!("error joining lobby: {e}");

                    error!(e);

                    if let Err(e) = sender
                        .send(WSMessagePass::Typed(ResponseType::Error { error: e }))
                        .await
                    {
                        error!("error sending error: {}", e);
                    }
                }

                let mesh = state.lock().await.lobbies.get(&lobby_id).map(|l| l.mesh);
                if let Some(mesh) = mesh
                    && let Err(e) = sender
                        .send(WSMessagePass::Typed(ResponseType::ID {
                            id: peer_id,
                            lobby_id: lobby_id.clone(),
                            mesh,
                        }))
                        .await
                {
                    error!("sending message to message passer through channel: {e}");
                }

                info!("joined lobby: {lobby_id}");
            }
            RequestType::Relay { dest_id, message } => {
                if let Err(e) = relay_message(peer_id, dest_id, message, state.clone()).await {
                    let e = format!("error relaying message: {e}");
                    error!(e);

                    if let Err(e) = sender
                        .send(WSMessagePass::Typed(ResponseType::Error { error: e }))
                        .await
                    {
                        error!("sending message to message passer through channel: {e}");
                    }
                }
            }
        };
    }
}

async fn create_lobby(mesh: bool, state: AppStateWrapped) -> String {
    let mut state = state.lock().await;

    let id = cuid2::CuidConstructor::new().with_length(8).create_id();

    let lobby = Lobby::new(id.clone(), mesh, vec![]);

    state.lobbies.insert(id.clone(), lobby);

    id
}

async fn relay_message(
    sender_id: i64,
    dest_id: i64,
    message: RelayMessage,
    state: AppStateWrapped,
) -> Result<(), Error> {
    let mut state = state.lock().await;

    let dest_lobby = match state.peers.get(&dest_id) {
        Some(p) => match &p.lobby {
            Some(l) => l,
            None => return Err(Error::NoLobby { peer_id: dest_id }),
        },
        None => return Err(Error::PeerDoesNotExist { id: dest_id }),
    };

    let sender_lobby = match state.peers.get(&sender_id) {
        Some(p) => match &p.lobby {
            Some(l) => l,
            None => return Err(Error::NoLobby { peer_id: dest_id }),
        },
        None => return Err(Error::PeerDoesNotExist { id: dest_id }),
    };

    if **dest_lobby != **sender_lobby {
        return Err(Error::LobbiesDoNotMatch {
            sender_lobby_id: (*sender_lobby).clone(),
            dest_lobby_id: (*dest_lobby).clone(),
        });
    }

    let lobby_id = dest_lobby.clone();

    let lobby = match state.lobbies.get_mut(&lobby_id) {
        Some(l) => l,
        None => {
            return Err(Error::LobbyDoesNotExist {
                id: lobby_id.clone(),
            });
        }
    };

    lobby
        .channel
        .send(ResponseType::Relay {
            sender_id,
            dest_id,
            message,
        })
        .map_err(Error::ChannelSendError)?;

    Ok(())
}

async fn join_lobby(peer_id: i64, lobby_id: String, state: AppStateWrapped) -> Result<(), Error> {
    let mut state = state.lock().await;

    let peer = match state.peers.get_mut(&peer_id) {
        Some(p) => p,
        None => return Err(Error::PeerDoesNotExist { id: peer_id }),
    };
    peer.lobby = Some(lobby_id.clone());

    let lobby = match state.lobbies.get_mut(&lobby_id) {
        Some(l) => l,
        None => return Err(Error::LobbyDoesNotExist { id: lobby_id }),
    };

    lobby.peers.push(peer_id);

    info!("channel count: {}", lobby.channel.strong_count());

    let e = lobby
        .channel
        .send(ResponseType::PeerConnect { id: peer_id })
        .map_err(Error::ChannelSendError);

    info!("after channel count: {}", lobby.channel.strong_count());
    e?;
    Ok(())
}
