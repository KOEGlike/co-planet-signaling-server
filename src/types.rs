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
    NoLobby{peer_id:i64},
    #[error("channel error: {0}")]
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

impl Eq for Lobby {
    
}


impl Lobby {
    pub fn new(id: String, mesh: bool, peers: Vec<i64>) -> Self {
        let (send, _) = broadcast::channel::<ResponseType>(100);

        Lobby {
            id,
            mesh,
            peers,
            channel: send,
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct AppState {
    pub lobbies: HashMap<String,Lobby>,
    pub peers: HashMap<i64,Peer>,
}

pub type AppStateWrapped = Arc<Mutex<AppState>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type",rename_all = "snake_case", rename_all_fields = "snake_case")]
pub enum RequestType {
    Join { lobby_id: Option<String> },
    Relay { dest_id: i64, message: RelayMessage },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type",rename_all = "snake_case", rename_all_fields = "snake_case")]
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
    Error{error:String},
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
