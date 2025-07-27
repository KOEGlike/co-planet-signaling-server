use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::{Mutex, broadcast};

#[derive(Error, Debug)]
pub enum Error {
    #[error("couldn't serialize json for response")]
    ServerSerialization(#[from] serde_json::Error),
    #[error("the lobby that you tried to join doesn't exist")]
    LobbyDoesNotExist,
    #[error("the player doesn't exist")]
    PeerDoesNotExist,
    #[error("channel error")]
    ChannelSendError(#[from] broadcast::error::SendError<ResponseType>),
}

#[derive(Clone, Debug)]
pub struct Peer {
    pub id: i64,
    pub lobby: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Lobby {
    pub id: String,
    pub mesh: bool,
    pub peers: Vec<i64>,
    pub channel: broadcast::Sender<ResponseType>,
}

impl Lobby {
    pub fn new(id: String, mesh: bool, peers: Vec<i64>) -> Self{
        let (send, _) = broadcast::channel::<ResponseType>(32);

        Lobby {
            id,
            mesh,
            peers,
            channel: send,
        }
    }
}

#[derive(Default)]
pub struct AppState {
    pub lobbies: Vec<Lobby>,
    pub peers: Vec<Peer>,
}

pub type AppStateWrapped = Arc<Mutex<AppState>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum RequestType {
    Join { lobby_id: Option<String> },
    Relay { dest_id: i64, message: RelayMessage },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum ResponseType {
    ID {
        id: i64,
        lobby_id: String,
        mesh: bool,
    },
    PeerConnect {
        id: i64,
    },
    PeerDisconnect {
        id: i64,
    },
    Relay {
        sender_id: i64,
        dest_id: i64,
        message: RelayMessage,
    },
    Error (String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type")]
pub enum RelayMessage {
    Offer(String),
    Answer(String),
    Candidate {
        mid: String,
        index: String,
        sdp: String,
    },
}
