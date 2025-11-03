use std::net::SocketAddr;

use anyhow::Result;
use axum::Router;
use dotenvy::dotenv;
use tracing::info;

use cvmsback::{api, config::AppConfig, db, telemetry};

#[tokio::main]
async fn main() -> Result<()> {
	// Load environment variables from .env if present
	dotenv().ok();
	telemetry::init_tracing();

	let cfg = AppConfig::from_env();
	let _pool = db::connect(&cfg.database_url).await?;

	let app: Router = api::router();

	let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;

	info!(%addr, "starting server");
	axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

	Ok(())
}
