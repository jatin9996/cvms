use std::net::SocketAddr;

use anyhow::Result;
use axum::Router;
use dotenvy::dotenv;
use tracing::info;

use cvmsback::{
    api, api::AppState, cache::Cache, config::AppConfig, db, metrics::Metrics, notify::Notifier, ops::RateLimiter,
    solana_client::SolanaClient, tasks, telemetry,
};

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
    
    // Initialize cache (optional, fails gracefully if Redis unavailable)
    let cache = Cache::new(&cfg.redis_url, cfg.cache_ttl_seconds)
        .map(|c| std::sync::Arc::new(c))
        .ok();
    if cache.is_some() {
        info!("Redis cache initialized");
    } else {
        info!("Redis cache unavailable, continuing without cache");
    }
    
    // Initialize metrics
    let metrics = Metrics::new().expect("Failed to initialize metrics");
    
    let state = AppState {
        pool: pool.clone(),
        cfg: cfg.clone(),
        sol,
        notifier: notifier.clone(),
        rate_limiter: rate_limiter.clone(),
        cache: cache.clone(),
        metrics: metrics.clone(),
    };

    // background tasks
    {
        let indexer_state = state.clone();
        let notifier = notifier.clone();
        tokio::spawn(async move {
            tasks::event_indexer::run_event_indexer(indexer_state, notifier.clone()).await;
        });
    }
    {
        let recon_state = state.clone();
        let notifier = notifier.clone();
        tokio::spawn(async move {
            tasks::reconciliation::run_reconciliation(recon_state.clone(), notifier.clone()).await;
        });
    }
    {
        let recon_state = state.clone();
        let notifier = notifier.clone();
        tokio::spawn(async move {
            tasks::monitor::run_monitor(recon_state.clone(), notifier.clone()).await;
        });
    }
    {
        let recon_state = state.clone();
        let notifier = notifier.clone();
        tokio::spawn(async move {
            tasks::timelocks::run_timelock_cron(recon_state.clone(), notifier.clone()).await;
        });
    }
    {
        let recon_state = state.clone();
        tokio::spawn(async move {
            tasks::yield_tasks::run_yield_scheduler(recon_state).await;
        });
    }
    {
        let balance_state = state.clone();
        let balance_notifier = notifier.clone();
        tokio::spawn(async move {
            tasks::balance_monitor::run_balance_monitor(
                balance_state,
                balance_notifier,
                cfg.balance_monitor_interval_seconds,
            )
            .await;
        });
    }

    let app: Router = api::router(state);

    let addr: SocketAddr = format!("{}:{}", cfg.host, cfg.port).parse()?;

    info!(%addr, "starting server");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;

    Ok(())
}
