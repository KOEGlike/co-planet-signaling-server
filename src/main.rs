use std::sync::{Arc};

use axum::{Router, routing::any};

use co_planet_signaling_server::*;

use tokio::sync::Mutex;
use tracing::info;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app_state: AppStateWrapped = Arc::new(Mutex::new(AppState::default()));

    let app = Router::new().route("/", any(ws_handler)).with_state(app_state);

    // run our app with hyper, listening globally on port 3000
    info!("listening on port 3000");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
