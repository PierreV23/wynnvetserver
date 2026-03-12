use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, OnceLock},
};
use tokio::sync::{Mutex, mpsc};
use uuid::Uuid;

use std::env;

// configuration
static REMOTE_API_URL: LazyLock<String> = LazyLock::new(|| {
    env::var("REMOTE_API_URL").unwrap_or_else(|_| "http://localhost:9421".to_string())
});

static API_SERVER_PORT: LazyLock<String> =
    LazyLock::new(|| env::var("API_SERVER_PORT").unwrap_or_else(|_| "9420".to_string()));
// honestly it's dumb i define these globally using lazylock, bcz i only use them once in the code and it causes annoying `&*` later

// registry is what will be storing the 'sender' and 'receiver' for a particular minecraft user

// define registry type
type Registry = Mutex<HashMap<Uuid, (mpsc::Sender<String>, Arc<Mutex<mpsc::Receiver<String>>>)>>;

// this is needed to get that "global state" functionality
static REGISTRY: OnceLock<Registry> = OnceLock::new();
fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

// register uuid to create channel, returns only rx
// can be called infinitely with the same uuid
pub async fn register(uuid: Uuid) -> Arc<Mutex<mpsc::Receiver<String>>> {
    let mut reg = registry().lock().await;
    if let Some((_, rx)) = reg.get(&uuid) {
        return rx.clone();
    }
    let (tx, rx) = mpsc::channel(16);
    let rx = Arc::new(Mutex::new(rx));
    reg.insert(uuid, (tx, rx.clone()));
    rx
}

// delete relevant register
pub async fn unregister(uuid: Uuid) {
    registry().lock().await.remove(&uuid);
}

// fires when a player sends a message in the minecraft chat
pub fn on_incoming_chat(uuid: Uuid, msg: String) {
    tokio::spawn(async move {
        let base_url = &*REMOTE_API_URL;
        let url = format!("{base_url}/{uuid}/{msg}");
        let _ = reqwest::get(&url).await;
    });
}

// http server:

//
pub async fn send_to_player(uuid: Uuid, msg: String) -> bool {
    if let Some((tx, _)) = registry().lock().await.get(&uuid) {
        tx.send(msg).await.is_ok()
    } else {
        false
    }
}

/// # Panics
pub fn start_http_server() {
    tokio::spawn(async {
        use axum::extract::Query;
        use axum::routing::get;
        use axum::{Router, http::StatusCode};
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct Params {
            uuid: Uuid,
            msg: String,
        }

        async fn send(Query(p): Query<Params>) -> StatusCode {
            if send_to_player(p.uuid, p.msg).await {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            }
        }

        let port = &*API_SERVER_PORT;
        let addr = format!("127.0.0.1:{port}");

        let app = Router::new().route("/send", get(send));
        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
        tracing::info!("HTTP Server on {addr}");
        axum::serve(listener, app).await.unwrap();
    });
}
