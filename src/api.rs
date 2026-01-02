use crate::db::Database;
use crate::settings::ServerSettings;
use axum::{
    extract::State,
    http::StatusCode,
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

pub struct AppState {
    pub db: Arc<Database>,
}

pub async fn run_server(settings: ServerSettings, db: Arc<Database>) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState { db });

    let app = Router::new()
        .route("/api/patchsets", get(list_patchsets))
        .route("/api/stats", get(get_stats))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], settings.port));
    info!("Web API listening on {}", addr);
    
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn list_patchsets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<crate::db::PatchsetRow>>, StatusCode> {
    match state.db.get_patchsets(50).await {
        Ok(patchsets) => Ok(Json(patchsets)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_stats() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": "0.1.0"
    }))
}
