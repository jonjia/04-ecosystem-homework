use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    extract::{Path, State},
    http::{header::LOCATION, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    serve, Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::info;

#[derive(Debug, Deserialize)]
struct ShortenRequest {
    url: String,
}

#[derive(Debug, Serialize)]
struct ShortenResponse {
    id: String,
}

#[derive(Clone)]
struct AppState {
    db: Arc<Mutex<HashMap<String, String>>>,
}

#[tokio::main]
async fn main() {
    let listen_addr = "0.0.0.0:3000";
    let state = AppState {
        db: Arc::new(Mutex::new(HashMap::new())),
    };
    let app = Router::new()
        .route("/", post(shorten))
        .route("/:id", get(redirect))
        .with_state(state);
    let server = TcpListener::bind(listen_addr).await.unwrap();

    info!("Listening on {}", listen_addr);

    serve(server, app).await.unwrap();
}

async fn shorten(
    State(state): State<AppState>,
    Json(request): Json<ShortenRequest>,
) -> impl IntoResponse {
    let id = state.shorten(request.url);
    Json(ShortenResponse { id })
}

async fn redirect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let url = state.get_url(&id);
    let mut headers = HeaderMap::new();
    match url {
        Some(url) => {
            headers.insert(LOCATION, url.parse().unwrap());
            Ok((headers, StatusCode::FOUND))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

impl AppState {
    fn shorten(&self, url: String) -> String {
        let id = nanoid::nanoid!(6);
        self.db.lock().unwrap().insert(id.clone(), url);
        id
    }

    fn get_url(&self, id: &String) -> Option<String> {
        self.db.lock().unwrap().get(id).cloned()
    }
}
