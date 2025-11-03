use std::net::SocketAddr;

use anyhow::Result;
use axum::Router;
use dotenvy::dotenv;
use tracing::info;

use cvmsback::{api, api::AppState, config::AppConfig, db, solana_client::SolanaClient, telemetry};

#[tokio::main]
async fn main() -> Result<()> {
	// Load environment variables from .env if present
	dotenv().ok();
	telemetry::init_tracing();

    let cfg = AppConfig::from_env();
    let pool = db::connect(&cfg.database_url).await?;
    let _ = db::init(&pool).await; // best effort to init tables

    let sol = SolanaClient::new(&cfg.solana_rpc_url);
    let state = AppState { pool: pool.clone(), cfg: cfg.clone(), sol };

    let app: Router = api::router(state);

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;

	info!(%addr, "starting server");
	axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

	Ok(())
}
