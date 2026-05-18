//! 3D graph visualizer.
//!
//! When compiled with the `visualizer` feature, serves an interactive 3D force-directed
//! graph on localhost:9749 using axum and an embedded HTML page with 3d-force-graph.
//!
//! Without the feature, provides a stub that prints a message.

use std::sync::Arc;

use crate::store::db::StoreManager;

/// Run the visualizer command (stub when feature is disabled).
///
/// Returns a message indicating the UI status.
pub fn run(ui_enabled: bool) -> String {
    if ui_enabled {
        #[cfg(feature = "visualizer")]
        {
            return "3D graph visualizer: ready to serve on http://localhost:9749".to_string();
        }
        #[cfg(not(feature = "visualizer"))]
        {
            return "3D graph visualizer: UI not available in this build. \
                 The visualizer requires the 'visualizer' feature to be enabled at compile time."
                .to_string();
        }
    } else {
        "Visualizer UI is disabled. Use --ui to enable the graph visualization server.".to_string()
    }
}

/// Start the visualizer HTTP server (feature-gated).
///
/// When the `visualizer` feature is enabled, this spawns an axum HTTP server
/// on the given port serving the 3D graph UI and API endpoints.
///
/// Without the feature, this is a no-op that logs a message.
#[cfg(feature = "visualizer")]
pub async fn serve(store: Arc<StoreManager>, port: u16) -> Result<(), anyhow::Error> {
    use axum::{
        extract::State,
        http::StatusCode,
        response::{Html, IntoResponse, Json},
        routing::get,
        Router,
    };
    use tower_http::cors::CorsLayer;

    static VISUALIZER_HTML: &str = include_str!("visualizer_html.html");

    #[derive(Clone)]
    struct AppState {
        store: Arc<StoreManager>,
    }

    async fn index_handler() -> Html<&'static str> {
        Html(VISUALIZER_HTML)
    }

    async fn health_handler() -> Json<serde_json::Value> {
        Json(serde_json::json!({"status": "ok"}))
    }

    async fn nodes_handler(State(state): State<AppState>) -> impl IntoResponse {
        let conn = state.store.read_conn();
        let mut stmt = match conn.prepare(
            "SELECT fqn, kind, file, start_line, end_line FROM nodes",
        ) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };

        let nodes: Vec<serde_json::Value> = match stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "fqn": row.get::<_, String>(0)?,
                "kind": row.get::<_, String>(1)?,
                "file": row.get::<_, String>(2)?,
                "start_line": row.get::<_, i64>(3)?,
                "end_line": row.get::<_, i64>(4)?,
            }))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        };

        Json(serde_json::json!(nodes)).into_response()
    }

    async fn edges_handler(State(state): State<AppState>) -> impl IntoResponse {
        let conn = state.store.read_conn();
        let mut stmt = match conn.prepare(
            "SELECT source_fqn, target_fqn, kind, confidence FROM edges",
        ) {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        };

        let edges: Vec<serde_json::Value> = match stmt.query_map([], |row| {
            Ok(serde_json::json!({
                "source_fqn": row.get::<_, String>(0)?,
                "target_fqn": row.get::<_, String>(1)?,
                "kind": row.get::<_, String>(2)?,
                "confidence": row.get::<_, f64>(3)?,
            }))
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        };

        Json(serde_json::json!(edges)).into_response()
    }

    async fn stats_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
        let conn = state.store.read_conn();

        let node_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .unwrap_or(0);

        let edge_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM edges", [], |row| row.get(0))
            .unwrap_or(0);

        let module_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM nodes WHERE kind = 'Module'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        Json(serde_json::json!({
            "node_count": node_count,
            "edge_count": edge_count,
            "module_count": module_count,
        }))
    }

    let state = AppState { store };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/health", get(health_handler))
        .route("/api/nodes", get(nodes_handler))
        .route("/api/edges", get(edges_handler))
        .route("/api/stats", get(stats_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("127.0.0.1:{}", port);
    tracing::info!("Visualizer HTTP server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Stub serve function when visualizer feature is not enabled.
#[cfg(not(feature = "visualizer"))]
pub async fn serve(_store: Arc<StoreManager>, _port: u16) -> Result<(), anyhow::Error> {
    tracing::info!("Visualizer feature not enabled. Compile with --features visualizer to enable.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visualizer_stub_returns_not_available() {
        let msg = run(true);
        assert!(msg.contains("not available") || msg.contains("ready to serve"));
    }

    #[test]
    fn test_visualizer_disabled_message() {
        let msg = run(false);
        assert!(msg.contains("disabled"));
    }
}
