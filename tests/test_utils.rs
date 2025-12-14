// Test utilities and helpers for end-to-end testing

use cvmsback::{
    api::AppState,
    config::AppConfig,
    db,
    notify::Notifier,
    ops::RateLimiter,
    solana_client::SolanaClient,
};
use solana_sdk::pubkey::Pubkey;
use sqlx::PgPool;

pub struct TestContext {
    pub state: AppState,
    pub pool: PgPool,
}

impl TestContext {
    pub async fn new() -> Self {
        // Use TEST_DATABASE_URL if set, otherwise fall back to DATABASE_URL from .env
        // This allows using the same database for development and testing
        let database_url = std::env::var("TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .unwrap_or_else(|_| "postgresql://postgres:5412@localhost:5432/cvms".to_string());
        
        let pool = db::connect(&database_url).await.expect("Failed to connect to test database");
        let _ = db::init(&pool).await.expect("Failed to initialize test database");

        let cfg = AppConfig {
            host: "0.0.0.0".to_string(),
            port: 8080,
            database_url: database_url.clone(),
            solana_rpc_url: std::env::var("TEST_SOLANA_RPC_URL")
                .unwrap_or_else(|_| "https://api.devnet.solana.com".to_string()),
            program_id: "5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT".to_string(),
            usdt_mint: "4QHVBbG3H8kbwvcSwPnze3sC91kdeYWxNf8S5hkZ9nbZ".to_string(),
            deployer_keypair_path: "".to_string(),
            vault_authority_pubkey: "".to_string(),
            admin_jwt_secret: "test_secret".to_string(),
            position_manager_program_id: "11111111111111111111111111111111".to_string(),
            reconciliation_threshold: 1000,
            low_balance_threshold: 10000,
            redis_url: std::env::var("TEST_REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".to_string()),
            cache_ttl_seconds: 60,
            balance_monitor_interval_seconds: 30,
        };

        let sol = SolanaClient::new(&cfg.solana_rpc_url);
        let notifier = Notifier::new(1024);
        let rate_limiter = std::sync::Arc::new(RateLimiter::new(100));
        
        let cache = cvmsback::cache::Cache::new(&cfg.redis_url, cfg.cache_ttl_seconds)
            .ok()
            .map(|c| std::sync::Arc::new(c));
        
        let metrics = cvmsback::metrics::Metrics::new()
            .expect("Failed to create metrics");

        let state = AppState {
            pool: pool.clone(),
            cfg,
            sol,
            notifier,
            rate_limiter,
            cache,
            metrics,
        };

        Self { state, pool }
    }

    pub fn generate_test_pubkey() -> Pubkey {
        Pubkey::new_unique()
    }

    pub fn generate_test_owner() -> String {
        Self::generate_test_pubkey().to_string()
    }

    pub async fn cleanup(&self) {
        // Clean up test data
        let _ = sqlx::query("DELETE FROM transactions").execute(&self.pool).await;
        let _ = sqlx::query("DELETE FROM vaults").execute(&self.pool).await;
        let _ = sqlx::query("DELETE FROM nonces").execute(&self.pool).await;
        let _ = sqlx::query("DELETE FROM audit_trail").execute(&self.pool).await;
    }
}

pub async fn create_test_vault(ctx: &TestContext, owner: &str) -> Result<(), sqlx::Error> {
    db::upsert_vault_token_account(&ctx.pool, owner, owner).await?;
    db::update_vault_snapshot(&ctx.pool, owner, 0, 0, 0).await?;
    Ok(())
}

pub async fn insert_test_transaction(
    ctx: &TestContext,
    owner: &str,
    signature: &str,
    amount: i64,
    kind: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    db::insert_transaction_with_status(&ctx.pool, owner, signature, Some(amount), kind, status).await
}

pub fn generate_test_signature() -> String {
    use solana_sdk::signature::Signature;
    Signature::new_unique().to_string()
}
