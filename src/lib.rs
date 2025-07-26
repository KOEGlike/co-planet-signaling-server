use std::{prelude::*, sync::Arc, vec};

use axum::{
    extract::{
        State,
        ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Response},
};

use serde::{Deserialize, Serialize};

use tokio::sync::{Mutex, broadcast};

use thiserror::Error;

use futures_util::{sink::SinkExt, stream::StreamExt};

#[derive(Error, Debug)]
enum Error {}

pub struct Peer {
    id: i128,
    lobby: i128,
}

pub struct Lobby {
    id: String,
    mesh: bool,
    peers: Vec<i128>,
    channel: broadcast::Sender<ResponseType>,
}

#[derive(Default)]
pub struct AppState {
    lobbies: Vec<Lobby>,
    peers: Vec<Peer>,
}

pub type AppStateWrapped = Arc<Mutex<AppState>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum RequestType {
    Join { lobby_id: Option<i128> },
    Relay { id: i128, message: RelayMessage },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum ResponseType {
    ID {
        id: i128,
        lobby_id: i128,
        mesh: bool,
    },
    PeerConnect {
        id: i128,
    },
    PeerDisconnect {
        id: i128,
    },
    Relay {
        id: i128,
        message: RelayMessage,
    },
    SuccessfulJoin {
        id: i128,
    },
    Error {
        code: i16,
        message: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
enum RelayMessage {
    Offer(String),
    Answer(String),
    Candidate {
        mid: String,
        index: String,
        sdp: String,
    },
}

pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppStateWrapped>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppStateWrapped) {
    let (mut sender, mut receiver) = socket.split();
    
    // while let Some(msg) = socket.recv().await {
    //     let msg = if let Ok(msg) = msg {
    //         msg
    //     } else {
    //         // client disconnected
    //         return;
    //     };

    //     if socket.send(msg).await.is_err() {
    //         // client disconnected
    //         return;
    //     }
    // }
}

async fn write(){

}

async fn create_lobby(mesh: bool, state: AppStateWrapped) -> String {
    let (send, _) = broadcast::channel::<ResponseType>(32);
    let id = cuid2::CuidConstructor::new().with_length(8).create_id();

    let mut state = (*state).lock().await;

    let lobby = Lobby {
        id: id.clone(),
        mesh,
        peers: vec![],
        channel: send,
    };

    state.lobbies.push(lobby);

    id
}
