use axum::{extract::{Path, State, TypedHeader}, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;

use crate::{
	auth::{verify_admin_jwt, verify_wallet_signature},
	config::AppConfig,
	db,
	error::AppError,
	solana_client::{
		build_compute_budget_instructions, build_instruction_deposit, build_instruction_initialize_vault,
		build_instruction_withdraw, load_deployer_keypair, DepositParams, WithdrawParams,
		build_instruction_pm_lock, build_instruction_pm_unlock, send_transaction_with_retries,
	},
};
use crate::{vault::VaultManager, cpi::CPIManager};
use super::AppState;

pub async fn health() -> Json<serde_json::Value> {
	Json(serde_json::json!({ "status": "ok" }))
}

pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
	let db_ok = sqlx::query_scalar!("SELECT 1 as one").fetch_one(&state.pool).await.is_ok();
	let rpc_ok = state.sol.rpc.get_latest_blockhash().await.is_ok();
	let status = if db_ok && rpc_ok { "ok" } else { "degraded" };
	(StatusCode::OK, Json(serde_json::json!({ "status": status, "db": db_ok, "rpc": rpc_ok })))
}

#[derive(Deserialize)]
pub struct InitializeVaultRequest { pub user_pubkey: String }

#[derive(Serialize)]
pub struct InitializeVaultResponse { pub payload: serde_json::Value }

pub async fn vault_initialize(State(state): State<AppState>, Json(req): Json<InitializeVaultRequest>) -> impl IntoResponse {
    let program_id = Pubkey::from_str(&state.cfg.program_id).unwrap_or(solana_sdk::pubkey::Pubkey::default());
    let user = match Pubkey::from_str(&req.user_pubkey) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid user_pubkey" })))
    };
    let ix = match build_instruction_initialize_vault(&program_id, &user) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
    };
    let accounts: Vec<serde_json::Value> = ix.accounts.iter().map(|a| serde_json::json!({
        "pubkey": a.pubkey.to_string(),
        "is_signer": a.is_signer,
        "is_writable": a.is_writable,
    })).collect();
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": accounts,
        "data": base64::encode(ix.data),
    });
    (Json(InitializeVaultResponse { payload }),)
}

#[derive(Deserialize)]
pub struct DepositRequest { pub owner: String, pub amount: u64, pub nonce: String, pub signature: String }

pub async fn vault_deposit(State(state): State<AppState>, Json(req): Json<DepositRequest>) -> impl IntoResponse {
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    // Verify wallet signature on message: deposit:{owner}:{amount}:{nonce}
    let message = format!("deposit:{}:{}:{}", req.owner, req.amount, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    // Consume nonce
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    let user = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let ix = match build_instruction_deposit(&DepositParams { program_id, user, amount: req.amount }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
    };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({
            "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable
        })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "deposit_request", serde_json::json!({ "amount": req.amount, "nonce": req.nonce })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

#[derive(Deserialize)]
pub struct WithdrawRequest { pub owner: String, pub amount: u64, pub nonce: String, pub signature: String }

pub async fn vault_withdraw(State(state): State<AppState>, Json(req): Json<WithdrawRequest>) -> impl IntoResponse {
    // Verify wallet signature on message: withdraw:{owner}:{amount}:{nonce}
    let message = format!("withdraw:{}:{}:{}", req.owner, req.amount, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    // Rate limiting per owner
    if !state.rate_limiter.check_and_record(&req.owner).await {
        return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({ "error": "rate limited" })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
    let withdraw_ix = match build_instruction_withdraw(&WithdrawParams { program_id, owner, amount: req.amount }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
    };
    ixs.push(withdraw_ix);

    // Build tx requiring only PDA signer or deployer as fee payer; here use deployer keypair as payer to submit
    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) {
        Ok(kp) => kp,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    };
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") })))
    };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);

    let sig = match crate::solana_client::send_transaction(&state.sol, &tx).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() })))
    };

    let _ = db::insert_transaction(&state.pool, &req.owner, &sig.to_string(), Some(req.amount as i64), "withdraw").await;
    // best-effort snapshot update via on-chain balance using owner as token account
    if let Ok(owner_pk) = Pubkey::from_str(&req.owner) {
        let _ = db::upsert_vault_token_account(&state.pool, &req.owner, &req.owner).await;
        if let Ok(chain_bal) = crate::solana_client::get_token_balance(&state.sol, &owner_pk).await {
            let _ = db::update_vault_snapshot(&state.pool, &req.owner, chain_bal as i64, 0, req.amount as i64).await;
            let _ = state.notifier.vault_balance_tx.send(serde_json::json!({
                "owner": req.owner,
                "balance": chain_bal,
            }).to_string());
        }
    }
    let _ = state.notifier.withdraw_tx.send(serde_json::json!({ "owner": req.owner, "amount": req.amount, "signature": sig.to_string() }).to_string());
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "withdraw_submitted", serde_json::json!({ "amount": req.amount, "signature": sig.to_string() })).await;
    (StatusCode::OK, Json(serde_json::json!({ "signature": sig.to_string() })))
}

pub async fn vault_balance(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    // Resolve owner -> token account via DB first; fallback to interpreting as token account
    if let Ok(Some((token_acc_opt, _))) = db::get_vault(&state.pool, &owner).await {
        if let Some(token_acc) = token_acc_opt {
            if let Ok(pk) = Pubkey::from_str(&token_acc) {
                match crate::solana_client::get_token_balance(&state.sol, &pk).await {
                    Ok(bal) => return (StatusCode::OK, Json(serde_json::json!({ "balance": bal }))),
                    Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() })))
                }
            }
        }
    }
    match Pubkey::from_str(&owner) {
        Ok(token_acc) => {
            match crate::solana_client::get_token_balance(&state.sol, &token_acc).await {
                Ok(bal) => (StatusCode::OK, Json(serde_json::json!({ "balance": bal }))),
                Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() })))
            }
        }
        Err(_) => (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner or token account" })))
    }
}

pub async fn vault_transactions(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    match db::list_transactions(&state.pool, &owner, 50, 0).await {
        Ok(rows) => {
            let items: Vec<_> = rows.into_iter().map(|(id, signature, amount, kind, created_at)| serde_json::json!({
                "id": id,
                "signature": signature,
                "amount": amount,
                "kind": kind,
                "created_at": created_at,
            })).collect();
            (StatusCode::OK, Json(serde_json::json!({ "items": items, "next": null })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

pub async fn vault_tvl(State(state): State<AppState>) -> impl IntoResponse {
    let row = sqlx::query_scalar!(
        "SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END), 0) AS tvl FROM transactions"
    ).fetch_one(&state.pool).await;
    match row {
        Ok(tvl) => {
            let _ = state.notifier.tvl_tx.send(serde_json::json!({ "tvl": tvl }).to_string());
            (StatusCode::OK, Json(serde_json::json!({ "tvl": tvl })) )
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

#[derive(Deserialize)]
pub struct NonceRequest { pub owner: String }

#[derive(Serialize)]
pub struct NonceResponse { pub nonce: String }

pub async fn issue_nonce(State(state): State<AppState>, Json(req): Json<NonceRequest>) -> impl IntoResponse {
    let nonce = uuid::Uuid::new_v4().to_string();
    match db::insert_nonce(&state.pool, &nonce, &req.owner).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "nonce": nonce }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

#[derive(Deserialize)]
pub struct AdminAddProgramRequest { pub program_id: String }

pub async fn admin_vault_authority_add(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<axum::headers::Authorization<axum::headers::authorization::Bearer>>,
    Json(req): Json<AdminAddProgramRequest>,
) -> impl IntoResponse {
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }
    if Pubkey::from_str(&req.program_id).is_err() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" })));
    }
    match db::add_authorized_program(&state.pool, &req.program_id).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}


#[derive(Deserialize)]
pub struct AdminSetVaultTokenAccountRequest { pub owner: String, pub token_account: String }

pub async fn admin_set_vault_token_account(
	State(state): State<AppState>,
	TypedHeader(auth): TypedHeader<axum::headers::Authorization<axum::headers::authorization::Bearer>>,
	Json(req): Json<AdminSetVaultTokenAccountRequest>,
) -> impl IntoResponse {
	let token = auth.token();
	if state.cfg.admin_jwt_secret.is_empty() {
		return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
	}
	if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
		return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
	}
	if Pubkey::from_str(&req.owner).is_err() || Pubkey::from_str(&req.token_account).is_err() {
		return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid pubkey(s)" })));
	}
	if let Err(e) = db::upsert_vault_token_account(&state.pool, &req.owner, &req.token_account).await {
		return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
	}
	// backfill snapshot from chain
	if let Ok(pk) = Pubkey::from_str(&req.token_account) {
		if let Ok(bal) = crate::solana_client::get_token_balance(&state.sol, &pk).await {
			let _ = db::update_vault_snapshot(&state.pool, &req.owner, bal as i64, 0, 0).await;
			let _ = state.notifier.vault_balance_tx.send(serde_json::json!({ "owner": req.owner, "balance": bal }).to_string());
		}
	}
	(StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}


#[derive(Deserialize)]
pub struct PmLockRequest { pub owner: String, pub amount: u64, pub nonce: String, pub signature: String }

pub async fn pm_lock(State(state): State<AppState>, Json(req): Json<PmLockRequest>) -> impl IntoResponse {
	let message = format!("pm_lock:{}:{}:{}", req.owner, req.amount, req.nonce);
	if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
		return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
	}
	match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
		Ok(true) => {},
		Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
		Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })) ),
	}
	let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
	let mgr = CPIManager::new(state.clone());
	match mgr.lock(&owner, req.amount).await {
		Ok(sig) => {
			let _ = db::insert_transaction(&state.pool, &req.owner, &sig, Some(req.amount as i64), "lock").await;
			let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "lock_submitted", serde_json::json!({ "amount": req.amount, "signature": sig })).await;
			let _ = state.notifier.lock_tx.send(serde_json::json!({"owner": req.owner, "amount": req.amount, "signature": sig}).to_string());
			(StatusCode::OK, Json(serde_json::json!({"signature": sig})))
		},
		Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()})))
	}
}

#[derive(Deserialize)]
pub struct PmUnlockRequest { pub owner: String, pub amount: u64, pub nonce: String, pub signature: String }

pub async fn pm_unlock(State(state): State<AppState>, Json(req): Json<PmUnlockRequest>) -> impl IntoResponse {
	let message = format!("pm_unlock:{}:{}:{}", req.owner, req.amount, req.nonce);
	if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
		return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
	}
	match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
		Ok(true) => {},
		Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
		Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })) ),
	}
	let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
	let mgr = CPIManager::new(state.clone());
	match mgr.unlock(&owner, req.amount).await {
		Ok(sig) => {
			let _ = db::insert_transaction(&state.pool, &req.owner, &sig, Some(req.amount as i64), "unlock").await;
			let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "unlock_submitted", serde_json::json!({ "amount": req.amount, "signature": sig })).await;
			let _ = state.notifier.unlock_tx.send(serde_json::json!({"owner": req.owner, "amount": req.amount, "signature": sig}).to_string());
			(StatusCode::OK, Json(serde_json::json!({"signature": sig})))
		},
		Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()})))
	}
}

