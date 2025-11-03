use std::net::SocketAddr;

use anyhow::Result;
use axum::{routing::get, Json, Router};
use dotenvy::dotenv;
use serde_json::json;
use tracing::{info, Level};
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

async fn health() -> Json<serde_json::Value> {
	Json(json!({ "status": "ok" }))
}

fn init_tracing() {
	let env_filter = EnvFilter::try_from_default_env()
		.or_else(|_| EnvFilter::try_new("info"))
		.unwrap_or_else(|_| EnvFilter::new(Level::INFO.as_str()));

	let fmt_layer = fmt::layer().with_target(false).with_level(true);
	let subscriber = Registry::default().with(env_filter).with(fmt_layer);
	tracing::subscriber::set_global_default(subscriber).ok();
}

#[tokio::main]
async fn main() -> Result<()> {
	// Load environment variables from .env if present
	dotenv().ok();
	init_tracing();

	let app = Router::new().route("/health", get(health));

	let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
	let port: u16 = std::env::var("PORT")
		.ok()
		.and_then(|p| p.parse().ok())
		.unwrap_or(8080);
	let addr: SocketAddr = format!("{}:{}", host, port).parse()?;

	info!(%addr, "starting server");
	axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

	Ok(())
}

fn main() {
    println!("Hello, world!");
}
