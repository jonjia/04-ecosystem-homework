use anyhow::Result;
use axum::{
    extract::{Path, State},
    http::{header::LOCATION, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    serve, Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tokio::net::TcpListener;
use tracing::info;

#[derive(Debug, Deserialize)]
struct ShortenRequest {
    url: String,
}

#[derive(Debug, Serialize)]
struct ShortenResponse {
    url: String,
}

#[derive(Clone)]
struct AppState {
    db: PgPool,
}

#[derive(FromRow)]
struct UrlRecord {
    #[sqlx(default)]
    id: String,
    #[sqlx(default)]
    url: String,
}

const BASE_URL: &str = "0.0.0.0:3000";

#[tokio::main]
async fn main() {
    let pool = PgPool::connect("postgres://postgres:password@localhost/shortener")
        .await
        .unwrap();
    let state = AppState { db: pool };
    let app = Router::new()
        .route("/", post(shorten))
        .route("/:id", get(redirect))
        .with_state(state);
    let server = TcpListener::bind(BASE_URL).await.unwrap();

    info!("Listening on {}", BASE_URL);

    serve(server, app).await.unwrap();
}

async fn shorten(
    State(state): State<AppState>,
    Json(request): Json<ShortenRequest>,
) -> impl IntoResponse {
    let id = state.shorten(request.url).await.unwrap();
    Json(ShortenResponse {
        url: format!("http://{}/{}", BASE_URL, id),
    })
}

async fn redirect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, StatusCode> {
    let url = state
        .get_url(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, url.parse().unwrap());
    Ok((headers, StatusCode::FOUND))
}

impl AppState {
    async fn shorten(&self, url: String) -> Result<String> {
        let id = nanoid::nanoid!(6);
        let ret: UrlRecord = sqlx::query_as(
            "INSERT INTO urls(id, url) VALUES ($1, $2) ON CONFLICT(url) DO UPDATE SET url=EXCLUDED.url RETURNING id;",
        )
        .bind(&id)
        .bind(url)
        .fetch_one(&self.db)
        .await?;
        Ok(ret.id)
    }

    async fn get_url(&self, id: &str) -> Result<String> {
        let record: UrlRecord = sqlx::query_as("SELECT url FROM urls WHERE id = $1;")
            .bind(id)
            .fetch_one(&self.db)
            .await?;
        Ok(record.url)
    }
}
