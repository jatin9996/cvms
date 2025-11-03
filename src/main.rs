use std::net::SocketAddr;

use anyhow::Result;
use axum::Router;
use dotenvy::dotenv;
use tracing::info;

use cvmsback::{api, api::AppState, config::AppConfig, db, solana_client::SolanaClient, telemetry, notify::Notifier, ops::RateLimiter, tasks};

#[tokio::main]
async fn main() -> Result<()> {
	// Load environment variables from .env if present
	dotenv().ok();
	telemetry::init_tracing();

    let cfg = AppConfig::from_env();
    let pool = db::connect(&cfg.database_url).await?;
    let _ = db::init(&pool).await; // best effort to init tables

    let sol = SolanaClient::new(&cfg.solana_rpc_url);
    let notifier = Notifier::new(1024);
    let rate_limiter = std::sync::Arc::new(RateLimiter::new(10));
    let state = AppState { pool: pool.clone(), cfg: cfg.clone(), sol, notifier: notifier.clone(), rate_limiter: rate_limiter.clone() };

    // background tasks
    let indexer_state = state.clone();
    let recon_state = state.clone();
    tokio::spawn(async move { tasks::event_indexer::run_event_indexer(indexer_state, notifier.clone()).await; });
    tokio::spawn(async move { tasks::reconciliation::run_reconciliation(recon_state, notifier.clone()).await; });

    let app: Router = api::router(state);

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;

	info!(%addr, "starting server");
	axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

	Ok(())
}
