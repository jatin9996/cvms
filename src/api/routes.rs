use axum::{extract::Path, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

pub async fn health() -> Json<serde_json::Value> {
	Json(serde_json::json!({ "status": "ok" }))
}

#[derive(Deserialize)]
pub struct InitializeVaultRequest {
	pub user_pubkey: String,
}

#[derive(Serialize)]
pub struct InitializeVaultResponse {
	pub payload: serde_json::Value,
}

pub async fn vault_initialize(Json(_req): Json<InitializeVaultRequest>) -> impl IntoResponse {
	(Json(InitializeVaultResponse { payload: serde_json::json!({}) }),)
}

#[derive(Deserialize)]
pub struct DepositRequest {
	pub owner: String,
	pub amount: u64,
}

pub async fn vault_deposit(Json(_req): Json<DepositRequest>) -> impl IntoResponse {
	(StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct WithdrawRequest {
	pub owner: String,
	pub amount: u64,
}

pub async fn vault_withdraw(Json(_req): Json<WithdrawRequest>) -> impl IntoResponse {
	(StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

pub async fn vault_balance(Path(_owner): Path<String>) -> impl IntoResponse {
	(StatusCode::OK, Json(serde_json::json!({ "balance": 0 })))
}

pub async fn vault_transactions(Path(_owner): Path<String>) -> impl IntoResponse {
	(StatusCode::OK, Json(serde_json::json!({ "items": [], "next": null })))
}

pub async fn vault_tvl() -> impl IntoResponse {
	(StatusCode::OK, Json(serde_json::json!({ "tvl": 0 })))
}


