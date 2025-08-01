#![allow(clippy::collapsible_if)]

use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    http::response,
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
use tracing::*;

pub mod types;
pub use types::*;

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppStateWrapped>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppStateWrapped) {
    let (mut sender, receiver) = socket.split();

    let (tx, mut rx) = mpsc::channel::<WSMessagePass>(100);

    let peer_id = rand::random_range(2..2147483647);

    info!("new socket connection, id:{peer_id}");

    state.lock().await.peers.insert(
        peer_id,
        Peer {
            id: peer_id,
            lobby: None,
        },
    );

    tokio::spawn(async move {
        let span = span!(Level::TRACE, "message passer");
        let _enter = span.enter();
        while let Some(msg) = rx.recv().await {
            let mut close = false;
            info!("sent message to {peer_id}: {msg:?}");
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
        let channel_receiver = {
            let mut state = state.lock().await;

            if let Some(p) = state.peers.get(&peer_id)
                && let Some(l_id) = p.lobby.clone()
                && let Some(lobby) = state.lobbies.get_mut(&l_id)
            {
                let channel = lobby.channel.subscribe();
                Some((l_id, lobby.host, channel))
            } else {
                None
            }
        };

        let (lobby_id, host, mut channel) = match channel_receiver {
            Some(ch) => ch,
            None => {
                sleep(Duration::from_millis(1)).await;
                continue;
            }
        };

        loop {
            let msg = channel.recv().await;

            let msg = match msg {
                Ok(m) => m,
                Err(broadcast::error::RecvError::Closed) => {
                    error!("channel closed for lobby {}", lobby_id);
                    sleep(Duration::from_millis(500)).await;
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(lag)) => {
                    warn!("lobby {} lagged {lag} messages", lobby_id);
                    continue;
                }
            };

            info!("got msg from channel: {msg:?}");

            let msg: WSMessagePass = match msg {
                WSMessagePass::Typed(ResponseType::Id { .. }) => continue,
                WSMessagePass::Typed(ResponseType::PeerConnect { id }) => {
                    if id != peer_id {
                        let id = if id == host { 1 } else { id };
                        ResponseType::PeerConnect { id }.into()
                    } else {
                        continue;
                    }
                }
                WSMessagePass::Typed(ResponseType::PeerDisconnect { id }) => {
                    if id != peer_id {
                        let id = if id == host { 1 } else { id };
                        ResponseType::PeerDisconnect { id }.into()
                    } else {
                        continue;
                    }
                }
                WSMessagePass::Typed(ResponseType::Relay {
                    sender_id: s,
                    dest_id,
                    message: ref m,
                }) => {
                    if let RelayMessage::Answer { .. } = m {
                        info!(
                            "got answer, peer {peer_id}, sender {s}, dest {dest_id}, message: {m:?}"
                        );
                    }
                    if dest_id == peer_id {
                        info!("sending relay to {peer_id}, message:{msg:?}");
                        msg
                    } else {
                        continue;
                    }
                }
                WSMessagePass::Typed(ResponseType::Error { .. }) => continue,
                _ => msg,
            };

            if let Err(e) = sender.send(msg).await {
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

        // info!("new ws message:{:#?}", msg);

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
                        .send(ResponseType::PeerDisconnect { id: peer_id }.into())
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

        // info!("text ws message: {msg}");

        let msg = serde_json::from_str::<RequestType>(&msg);

        let msg = match msg {
            Ok(msg) => msg,
            Err(e) => {
                error! {"error deserializing request json {}", e};
                continue;
            }
        };

        info!("deserialized msg: {:?}", msg);

        match msg {
            RequestType::Join { lobby_id } => {
                let lobby_id = match lobby_id {
                    LobbyId::Existing { id } => id,
                    LobbyId::Create { mesh } => create_lobby(peer_id, mesh, state.clone()).await,
                };

                // info!("lobby id: {lobby_id}");

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

                let state = state.lock().await;
                let lobby = state.lobbies.get(&lobby_id);

                if let Some(lobby) = lobby {
                    if let Err(e) = sender
                        .send(WSMessagePass::Typed(ResponseType::Id {
                            id: { if lobby.host == peer_id { 1 } else { peer_id } },
                            lobby_id: lobby_id.clone(),
                            mesh: lobby.mesh,
                        }))
                        .await
                    {
                        error!("sending message to message passer through channel: {e}");
                    }

                    for p in &lobby.peers {
                        if *p != peer_id {
                            if let Err(e) = sender
                                .send(WSMessagePass::Typed(ResponseType::PeerConnect {
                                    id: if *p == lobby.host { 1 } else { *p },
                                }))
                                .await
                            {
                                error!("sending message to message passer through channel: {e}");
                            }
                        }
                    }
                }

                info!("joined lobby: {lobby_id}, peer {peer_id}");
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

    let mut state = state.lock().await;
    if let Some(peer) = state.peers.get(&peer_id).cloned() {
        state.peers.remove(&peer_id);

        let lobby = if let Some(ref lobby_id) = peer.lobby
            && let Some(lobby) = state.lobbies.get_mut(lobby_id)
        {
            lobby
        } else {
            return;
        };

        let pos = lobby.peers.iter().position(|p| *p == peer.id);

        if let Some(pos) = pos {
            lobby.peers.remove(pos);
        }

        if lobby.host == peer_id {
            if let Err(e) = sender
                .send(WSMessagePass::Raw(Message::Close(Some(CloseFrame {
                    code: 1011,
                    reason: Utf8Bytes::from_static("host disconnected"),
                }))))
                .await
            {
                error!("error sending closing message to peers: {e}s");
            }

            for p in lobby.peers.clone() {
                state.peers.remove(&p);
            }
        }

        
    }
}

async fn create_lobby(host: i64, mesh: bool, state: AppStateWrapped) -> String {
    let mut state = state.lock().await;

    let id = cuid2::CuidConstructor::new().with_length(8).create_id();

    let lobby = Lobby::new(id.clone(), host, mesh, vec![]);

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

    let sender_lobby = match state.peers.get(&sender_id) {
        Some(p) => match &p.lobby {
            Some(l) => l,
            None => return Err(Error::NoLobby { peer_id: dest_id }),
        },
        None => return Err(Error::PeerDoesNotExist { id: dest_id }),
    };

    if dest_id != 1 {
        let dest_lobby = match state.peers.get(&dest_id) {
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
    }

    let lobby_id = sender_lobby.clone();

    let lobby = match state.lobbies.get_mut(&lobby_id) {
        Some(l) => l,
        None => {
            return Err(Error::LobbyDoesNotExist {
                id: lobby_id.clone(),
            });
        }
    };

    let sender_id = if sender_id == lobby.host {
        1
    } else {
        sender_id
    };

    let dest_id = if dest_id == 1 { lobby.host } else { dest_id };

    lobby
        .channel
        .send(
            ResponseType::Relay {
                sender_id,
                dest_id,
                message,
            }
            .into(),
        )
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

    lobby
        .channel
        .send(ResponseType::PeerConnect { id: peer_id }.into())
        .map_err(Error::ChannelSendError)?;

    Ok(())
}
