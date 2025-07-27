use axum::extract::ws::*;
use crate::AppStateWrapped;
use futures_util::StreamExt;

async fn handleeee_socket(mut socket: WebSocket) {
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