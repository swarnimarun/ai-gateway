use axum::{extract::Request, response::IntoResponse, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dummyai=debug,tower_http=trace,axum::rejection=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app = Router::new()
        .route("/", get(root))
        .route("/foo", get(root))
        .route("/test", get(root));

    // run our app with hyper, listening globally on port 8000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await.unwrap();
    tracing::info!("Listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

#[derive(Debug, Deserialize, Serialize)]
struct Message {
    role: String,
    content: String,
}

// basic handler that responds with a static string
async fn root(req: Request) -> impl IntoResponse {
    tracing::info!(
        "Request: {} {}",
        req.method(),
        req.uri().path_and_query().unwrap()
    );
    Json(Message {
        role: "assitant".to_string(),
        content: "This is some text that most people won't read.".to_string(),
    })
}
