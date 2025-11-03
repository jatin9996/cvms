use axum::{routing::{get, post}, Router};
use sqlx::PgPool;

use crate::{config::AppConfig, solana_client::SolanaClient};

mod routes;
mod ws;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: AppConfig,
    pub sol: SolanaClient,
}

pub fn router(state: AppState) -> Router {
    Router::new()
		.route("/health", get(routes::health))
        .route("/auth/nonce", post(routes::issue_nonce))
		.route("/vault/initialize", post(routes::vault_initialize))
		.route("/vault/deposit", post(routes::vault_deposit))
		.route("/vault/withdraw", post(routes::vault_withdraw))
		.route("/vault/balance/:owner", get(routes::vault_balance))
		.route("/vault/transactions/:owner", get(routes::vault_transactions))
		.route("/vault/tvl", get(routes::vault_tvl))
        .route("/admin/vault-authority/add", post(routes::admin_vault_authority_add))
		.route("/ws", get(ws::ws_handler))
        .with_state(state)
}


