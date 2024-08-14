use std::time::Duration;

use anyhow::Result;
use axum::{
    error_handling::HandleErrorLayer,
    extract::{rejection::JsonRejection, Path, State},
    http::{header::LOCATION, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    serve, BoxError, Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use tokio::net::TcpListener;
use tower::{timeout::error::Elapsed, ServiceBuilder};
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::{fmt::Layer, layer::SubscriberExt, util::SubscriberInitExt, Layer as _};

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

#[derive(Debug, Error)]
enum AppError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse JSON: {0}")]
    JsonRejection(#[from] JsonRejection),

    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),

    #[error("Timeout error: {0}, request took too long, max time is 1ms")]
    Timeout(#[from] Elapsed),

    #[error("Internal server error {0}")]
    InternalServer(#[from] anyhow::Error),
}

const BASE_URL: &str = "0.0.0.0:9876";

#[tokio::main]
async fn main() -> Result<()> {
    let layer = Layer::new().pretty().with_filter(LevelFilter::INFO);
    tracing_subscriber::registry().with(layer).init();

    let db_url = "postgres://postgres:password@localhost/shortener";
    let state = AppState::try_new(db_url).await?;
    info!("Connected to database, {}", db_url);

    let listener = TcpListener::bind(BASE_URL).await.map_err(AppError::Io)?;
    info!("Listening on {}", BASE_URL);

    let router = Router::new()
        .route("/", post(shorten))
        .route("/:id", get(redirect))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_timeout_error))
                .timeout(Duration::from_secs(30)),
        )
        .with_state(state);

    serve(listener, router).await?;

    Ok(())
}

async fn shorten(
    State(state): State<AppState>,
    Json(data): Json<ShortenRequest>,
) -> Result<impl IntoResponse, AppError> {
    let id = state
        .shorten(&data.url)
        .await
        .map_err(AppError::InternalServer)?;
    let body = Json(ShortenResponse {
        url: format!("http://{}/{}", BASE_URL, id),
    });

    Ok((StatusCode::CREATED, body))
}

async fn redirect(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let url = state.get_url(&id).await.map_err(AppError::InternalServer)?;
    let mut headers = HeaderMap::new();
    headers.insert(LOCATION, url.parse().unwrap());
    Ok((StatusCode::PERMANENT_REDIRECT, headers))
}

async fn handle_timeout_error(err: BoxError) -> Result<(), AppError> {
    if err.is::<Elapsed>() {
        Err(AppError::Timeout(Elapsed::new()))
    } else {
        Err(AppError::InternalServer(anyhow::Error::msg(
            "Internal server error",
        )))
    }
}

impl AppState {
    async fn try_new(url: &str) -> Result<Self, AppError> {
        let pool = PgPool::connect(url).await?;
        // create table if not exists
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS urls (
                id CHAR(6) PRIMARY KEY,
                url TEXT NOT NULL UNIQUE
            );"#,
        )
        .execute(&pool)
        .await?;

        Ok(Self { db: pool })
    }

    async fn shorten(&self, url: &str) -> Result<String> {
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

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::JsonRejection(rejection) => (rejection.status(), rejection.body_text()),
            AppError::Timeout(err) => (StatusCode::REQUEST_TIMEOUT, err.to_string()),
            AppError::InternalServer(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_owned(),
            ),
        };

        (status, message).into_response()
    }
}
