use axum::extract::ws::Message;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, hash::Hash, sync::Arc};
use thiserror::Error;
use tokio::sync::{Mutex, broadcast};

#[derive(Error, Debug)]
pub enum Error {
    #[error("couldn't serialize json for response: {0}")]
    ServerSerialization(#[from] serde_json::Error),
    #[error("the lobby with the id of {id} to join doesn't exist")]
    LobbyDoesNotExist { id: String },
    #[error("the player with the id of {id} doesn't exist")]
    PeerDoesNotExist { id: i64 },
    #[error(
        "the player that you tried to relay to isn't in the lobby with the id of {sender_lobby_id}, it's in {dest_lobby_id}"
    )]
    LobbiesDoNotMatch {
        sender_lobby_id: String,
        dest_lobby_id: String,
    },
    #[error("the peer with the id of {peer_id} has no lobby")]
    NoLobby { peer_id: i64 },
    #[error("channel error: {0}")]
    ChannelSendError(#[from] broadcast::error::SendError<WSMessagePass>),
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
    pub host: i64,
    pub channel: broadcast::Sender<WSMessagePass>,
}

impl Hash for Lobby {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.mesh.hash(state);
        self.peers.hash(state);
    }
}

impl PartialEq for Lobby {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for Lobby {}

impl Lobby {
    pub fn new(id: String, host: i64, mesh: bool, peers: Vec<i64>) -> Self {
        let (send, _) = broadcast::channel::<WSMessagePass>(100);

        Lobby {
            id,
            host,
            mesh,
            peers,
            channel: send,
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct AppState {
    pub lobbies: HashMap<String, Lobby>,
    pub peers: HashMap<i64, Peer>,
}

pub type AppStateWrapped = Arc<Mutex<AppState>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case"
)]
pub enum LobbyId {
    Existing { id: String },
    Create { mesh: bool },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case"
)]
pub enum RequestType {
    Join { lobby_id: LobbyId },
    Relay { dest_id: i64, message: RelayMessage },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case"
)]
pub enum ResponseType {
    Id {
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
    Error {
        error: String,
    },
}

#[derive(Debug, Clone)]
pub enum WSMessagePass {
    Raw(Message),
    Typed(ResponseType),
}

impl Into<WSMessagePass> for ResponseType {
    fn into(self) -> WSMessagePass {
        WSMessagePass::Typed(self)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case"
)]
pub enum RelayMessage {
    Offer {
        offer: String,
    },
    Answer {
        answer: String,
    },
    Candidate {
        mid: String,
        index: i64,
        sdp: String,
    },
}

#[derive(Debug, Clone)]

pub enum ChannelCommunionMessage {
    Close {code:u16, reason: String },
}
