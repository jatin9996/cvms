use axum::{routing::{get, post, delete}, Router};
use tower_governor::{GovernorConfigBuilder, GovernorLayer};
use sqlx::PgPool;

use crate::{config::AppConfig, solana_client::SolanaClient, notify::Notifier, ops::RateLimiter};

mod routes;
mod ws;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cfg: AppConfig,
    pub sol: SolanaClient,
    pub notifier: std::sync::Arc<Notifier>,
    pub rate_limiter: std::sync::Arc<RateLimiter>,
}

pub fn router(state: AppState) -> Router {
    let governor_conf = Box::leak(Box::new(GovernorConfigBuilder::default()
        .per_second(5)
        .burst_size(10)
        .finish()
        .expect("governor config")));

    Router::new()
		.route("/health", get(routes::health))
		.route("/ready", get(routes::ready))
        .route("/auth/nonce", post(routes::issue_nonce))
		.route("/vault/initialize", post(routes::vault_initialize))
		.route("/vault/deposit", post(routes::vault_deposit))
		.route("/vault/withdraw", post(routes::vault_withdraw).route_layer(GovernorLayer::new(governor_conf)))
        .route("/vault/schedule-withdraw", post(routes::vault_schedule_withdraw).route_layer(GovernorLayer::new(governor_conf)))
        .route("/vault/emergency-withdraw", post(routes::vault_emergency_withdraw))
        .route("/vault/config/:owner", get(routes::vault_config))
        .route("/vault/timelocks/:owner", get(routes::vault_list_timelocks))
        .route("/vault/propose-withdraw", post(routes::vault_propose_withdraw))
        .route("/vault/approve-withdraw", post(routes::vault_approve_withdraw))
        .route("/vault/proposal/:id", get(routes::vault_proposal_status))
		.route("/vault/balance/:owner", get(routes::vault_balance))
		.route("/vault/transactions/:owner", get(routes::vault_transactions))
		.route("/vault/tvl", get(routes::vault_tvl))
        .route("/vault/yield-status/:owner", get(routes::vault_yield_status))
        .route("/vault/yield-deposit", post(routes::vault_yield_deposit))
        .route("/vault/yield-withdraw", post(routes::vault_yield_withdraw))
        .route("/vault/compound", post(routes::vault_compound_yield))
        // Withdraw policy & whitelist admin
        .route("/admin/withdraw/whitelist/add", post(routes::admin_withdraw_whitelist_add))
        .route("/admin/withdraw/whitelist/remove", delete(routes::admin_withdraw_whitelist_remove))
        .route("/admin/withdraw/min-delay/set", post(routes::admin_withdraw_min_delay_set))
        .route("/admin/withdraw/rate-limit/set", post(routes::admin_withdraw_rate_limit_set))
        // 2FA endpoints
        .route("/2fa/setup", post(routes::twofa_setup))
        .route("/2fa/verify", post(routes::twofa_verify))
        // Limits & analytics
        .route("/vault/limits/:owner", get(routes::vault_limits))
        .route("/analytics/tvl-series", get(routes::analytics_tvl_series))
        .route("/analytics/distribution", get(routes::analytics_distribution))
        .route("/analytics/utilization", get(routes::analytics_utilization))
        // Withdraw request (min-delay flow)
        .route("/vault/request-withdraw", post(routes::vault_request_withdraw))
        .route("/vault/delegate/add", post(routes::vault_delegate_add))
        .route("/vault/delegate/remove", delete(routes::vault_delegate_remove))
        .route("/admin/vault-authority/add", post(routes::admin_vault_authority_add))
        .route("/admin/yield-program/add", post(routes::admin_yield_program_add))
        .route("/admin/yield-program/remove", post(routes::admin_yield_program_remove))
        .route("/admin/risk-level/set", post(routes::admin_risk_level_set))
        .route("/admin/vault-token-account/set", post(routes::admin_set_vault_token_account))
		.route("/pm/lock", post(routes::pm_lock).route_layer(GovernorLayer::new(governor_conf)))
		.route("/pm/unlock", post(routes::pm_unlock).route_layer(GovernorLayer::new(governor_conf)))
		.route("/ws", get(ws::ws_handler))
        .with_state(state)
}


