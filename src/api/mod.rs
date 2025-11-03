use axum::{routing::{get, post}, Router};

mod routes;
mod ws;

pub fn router() -> Router {
	Router::new()
		.route("/health", get(routes::health))
		.route("/vault/initialize", post(routes::vault_initialize))
		.route("/vault/deposit", post(routes::vault_deposit))
		.route("/vault/withdraw", post(routes::vault_withdraw))
		.route("/vault/balance/:owner", get(routes::vault_balance))
		.route("/vault/transactions/:owner", get(routes::vault_transactions))
		.route("/vault/tvl", get(routes::vault_tvl))
		.route("/ws", get(ws::ws_handler))
}


