use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::time::Duration;
use tokio_too_busy::*;

#[derive(Clone)]
struct AppState {
    too_busy: TooBusy,
}

async fn my_middleware(State(state): State<AppState>, request: Request, next: Next) -> Response {
    if state.too_busy.eval() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Server is too busy").into_response();
    }
    next.run(request).await
}

async fn root() -> &'static str {
    "Hello, World!"
}

#[tokio::main(flavor = "multi_thread", worker_threads = 1)]
async fn main() {
    let too_busy = TooBusy::builder()
        .interval(Duration::from_secs(1))
        .ewma_alpha(0.1)
        .high_watermark(95)
        .low_watermark(90)
        .build();

    let state = AppState { too_busy };

    let app = Router::new()
        .route("/", get(root))
        .route_layer(middleware::from_fn_with_state(state.clone(), my_middleware))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
