use axum::{extract::{Path, State}, http::{StatusCode, HeaderMap}, response::IntoResponse, Json};
use axum_extra::{
    extract::TypedHeader,
    headers::{authorization::Bearer, Authorization},
};
use serde::{Deserialize, Serialize};
use solana_sdk::{pubkey::Pubkey, signature::Signer};
use std::str::FromStr;

use crate::{
	auth::{verify_admin_jwt, verify_wallet_signature},
	config::AppConfig,
	db,
	error::AppError,
	solana_client::{
        build_compute_budget_instructions, build_instruction_deposit, build_instruction_initialize_vault,
        build_instruction_withdraw, build_instruction_withdraw_multisig, build_instruction_schedule_timelock, load_deployer_keypair, DepositParams, WithdrawParams,
        fetch_vault_multisig_config, build_partial_withdraw_tx, WithdrawMultisigParams,
		build_instruction_pm_lock, build_instruction_pm_unlock, send_transaction_with_retries,
        build_instruction_emergency_withdraw, EmergencyWithdrawParams,
        build_instruction_yield_deposit, build_instruction_yield_withdraw, build_instruction_compound_yield,
        YieldDepositParams, YieldWithdrawParams, CompoundYieldParams,
        build_instruction_set_withdraw_min_delay, build_instruction_set_withdraw_rate_limit,
        build_instruction_add_withdraw_whitelist, build_instruction_remove_withdraw_whitelist,
        build_instruction_request_withdraw,
        build_instruction_transfer_collateral, TransferCollateralParams,
        build_instruction_add_yield_program, build_instruction_remove_yield_program,
        build_instruction_set_risk_level, AddYieldProgramParams, RemoveYieldProgramParams, SetRiskLevelParams,
	},
};
use crate::{vault::VaultManager, cpi::CPIManager};
use super::AppState;

pub async fn health() -> Json<serde_json::Value> {
	Json(serde_json::json!({ "status": "ok" }))
}

pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
	let db_ok = sqlx::query_scalar::<_, i64>("SELECT 1 as one").fetch_one(&state.pool).await.is_ok();
	let rpc_ok = state.sol.rpc.get_latest_blockhash().await.is_ok();
	let status = if db_ok && rpc_ok { "ok" } else { "degraded" };
	(StatusCode::OK, Json(serde_json::json!({ "status": status, "db": db_ok, "rpc": rpc_ok })))
}

#[derive(Deserialize)]
pub struct InitializeVaultRequest { pub user_pubkey: String }

pub async fn vault_initialize(State(state): State<AppState>, Json(req): Json<InitializeVaultRequest>) -> impl IntoResponse {
    let program_id = Pubkey::from_str(&state.cfg.program_id).unwrap_or(solana_sdk::pubkey::Pubkey::default());
    let user = match Pubkey::from_str(&req.user_pubkey) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid user_pubkey" })))
    };
    let usdt_mint = match Pubkey::from_str(&state.cfg.usdt_mint) {
        Ok(p) => p,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid usdt mint" })))
    };
    let ix = match build_instruction_initialize_vault(&program_id, &user, &usdt_mint) {
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
    (StatusCode::OK, Json(serde_json::json!({ "payload": payload })))
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let mint = match Pubkey::from_str(&state.cfg.usdt_mint) { Ok(p) => p, Err(_) => Pubkey::default() };
    let ix = match build_instruction_deposit(&DepositParams { program_id, owner, mint, amount: req.amount }) {
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

pub async fn vault_withdraw(State(state): State<AppState>, headers: HeaderMap, Json(req): Json<WithdrawRequest>) -> impl IntoResponse {
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    // Enforce 2FA if enabled for owner
    if let Ok(Some((secret, enabled))) = db::twofa_get(&state.pool, &req.owner).await {
        if enabled {
            // Expect client to include header X-2FA-CODE
            if let Some(val) = headers.get("x-2fa-code").and_then(|v| v.to_str().ok()) {
                if !crate::security::verify_totp(&secret, val) {
                    return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "invalid 2fa code" })));
                }
            } else {
                return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "2fa code required" })));
            }
        }
    }

    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let mint = match Pubkey::from_str(&state.cfg.usdt_mint) { Ok(p) => p, Err(_) => Pubkey::default() };
    let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
    let withdraw_ix = match build_instruction_withdraw(&WithdrawParams { program_id, owner, mint, amount: req.amount }) {
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

    let sig = match send_transaction_with_retries(&state.sol, &tx, 3).await {
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
                    Ok(bal) => return (StatusCode::OK, Json(serde_json::json!({ "balance": bal })) ),
                    Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() })))
                }
            }
        }
    }
    match Pubkey::from_str(&owner) {
        Ok(token_acc) => {
            match crate::solana_client::get_token_balance(&state.sol, &token_acc).await {
                Ok(bal) => (StatusCode::OK, Json(serde_json::json!({ "balance": bal })) ),
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

#[derive(Deserialize)]
pub struct ScheduleWithdrawRequest { pub owner: String, pub amount: u64, pub duration_seconds: i64, pub nonce: String, pub signature: String }

pub async fn vault_schedule_withdraw(State(state): State<AppState>, Json(req): Json<ScheduleWithdrawRequest>) -> impl IntoResponse {
    // Verify signature on message: schedule:{owner}:{amount}:{duration}:{nonce}
    let message = format!("schedule:{}:{}:{}:{}", req.owner, req.amount, req.duration_seconds, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    if req.duration_seconds < 0 { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid duration" }))); }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })) ),
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let mut ixs = build_compute_budget_instructions(1_200_000, 1_000);
    let sched_ix = match build_instruction_schedule_timelock(&crate::solana_client::ScheduleTimelockParams { program_id, owner, amount: req.amount, duration_seconds: req.duration_seconds }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
    };
    ixs.push(sched_ix);
    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) { Ok(kp) => kp, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))) };
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await { Ok(h) => h, Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") }))) };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
    let sig = match crate::solana_client::send_transaction(&state.sol, &tx).await { Ok(s) => s, Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))) };

    // Insert timelock row for UI and cron
    let unlock_at = time::OffsetDateTime::now_utc() + time::Duration::seconds(req.duration_seconds);
    let _ = db::timelock_insert(&state.pool, &req.owner, req.amount as i64, unlock_at).await;
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "timelock_scheduled", serde_json::json!({ "amount": req.amount, "unlock_at": unlock_at })).await;
    let _ = state.notifier.timelock_tx.send(serde_json::json!({ "owner": req.owner, "amount": req.amount, "unlock_at": unlock_at, "signature": sig.to_string() }).to_string());
    (StatusCode::OK, Json(serde_json::json!({ "signature": sig.to_string(), "unlock_at": unlock_at })))
}

pub async fn vault_list_timelocks(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    match db::timelock_list(&state.pool, &owner).await {
        Ok(rows) => {
            let now = time::OffsetDateTime::now_utc();
            let items: Vec<_> = rows.into_iter().map(|(id, amount, unlock_at, status)| {
                let remaining = (unlock_at - now).whole_seconds();
                serde_json::json!({ "id": id, "amount": amount, "unlock_at": unlock_at, "status": status, "remaining_seconds": remaining.max(0) })
            }).collect();
            (StatusCode::OK, Json(serde_json::json!({ "items": items })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

pub async fn vault_tvl(State(state): State<AppState>) -> impl IntoResponse {
    let row = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END), 0) AS tvl FROM transactions"
    );
    let row = row.fetch_one(&state.pool).await;
    match row {
        Ok(tvl) => {
            let _ = state.notifier.tvl_tx.send(serde_json::json!({ "tvl": tvl }).to_string());
            (StatusCode::OK, Json(serde_json::json!({ "tvl": tvl })) )
        },
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

#[derive(Deserialize)]
pub struct EmergencyWithdrawRequest { pub owner: String, pub amount: u64, pub reason: Option<String> }

pub async fn vault_emergency_withdraw(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(req): Json<EmergencyWithdrawRequest>,
) -> impl IntoResponse {
    // Admin protection
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }

    // Build instruction with governance authority (deployer keypair assumed to be governance signer)
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))) };

    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) {
        Ok(kp) => kp,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    };

    // Optional sanity: ensure payer pubkey matches configured vault authority if provided
    if !state.cfg.vault_authority_pubkey.is_empty() {
        if let Ok(cfg_va) = Pubkey::from_str(&state.cfg.vault_authority_pubkey) {
            if cfg_va != payer.pubkey() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "payer is not governance signer" })));
            }
        }
    }

    let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
    let mint = match Pubkey::from_str(&state.cfg.usdt_mint) { Ok(p) => p, Err(_) => Pubkey::default() };
    let em_ix = match build_instruction_emergency_withdraw(&EmergencyWithdrawParams { program_id, authority: payer.pubkey(), owner, amount: req.amount }, &mint) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() })))
    };
    ixs.push(em_ix);

    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") })))
    };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);

    let sig = match crate::solana_client::send_transaction(&state.sol, &tx).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() })))
    };

    let _ = db::insert_transaction(&state.pool, &req.owner, &sig.to_string(), Some(req.amount as i64), "emergency_withdraw").await;
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "emergency_withdraw", serde_json::json!({ "amount": req.amount, "reason": req.reason })).await;
    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "emergency_withdraw", "owner": req.owner, "amount": req.amount, "signature": sig.to_string() }).to_string());
    (StatusCode::OK, Json(serde_json::json!({ "signature": sig.to_string() })))
}

// --------------
// Vault config IO
// --------------
pub async fn vault_config(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner_pk = match Pubkey::from_str(&owner) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))) };
    let ms = fetch_vault_multisig_config(&state.sol, &owner_pk, &program_id).await;
    let delegates = db::delegate_list(&state.pool, &owner).await.unwrap_or_default();
    match ms {
        Ok((threshold, signers)) => {
            let signers_str: Vec<String> = signers.into_iter().map(|p| p.to_string()).collect();
            (StatusCode::OK, Json(serde_json::json!({ "threshold": threshold, "signers": signers_str, "delegates": delegates })))
        }
        Err(e) => (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string(), "delegates": delegates })))
    }
}

// -----------------
// Multisig endpoints
// -----------------
#[derive(Deserialize)]
pub struct ProposeWithdrawRequest { pub owner: String, pub amount: u64, pub threshold: u8, pub signers: Vec<String>, pub nonce: String, pub signature: String }

pub async fn vault_propose_withdraw(State(state): State<AppState>, Json(req): Json<ProposeWithdrawRequest>) -> impl IntoResponse {
    // Verify initiator signature on message: propose_withdraw:{owner}:{amount}:{threshold}:{nonce}
    let message = format!("propose_withdraw:{}:{}:{}:{}", req.owner, req.amount, req.threshold, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    let (threshold, signers) = if req.signers.is_empty() || req.threshold == 0 {
        // auto-fetch config from chain
        let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
        let owner_pk = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
        match fetch_vault_multisig_config(&state.sol, &owner_pk, &program_id).await {
            Ok((t, ss)) => (t, ss.into_iter().map(|p| p.to_string()).collect::<Vec<_>>()),
            Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))),
        }
    } else {
        // use provided
        // uniq signers sanity
        let mut uniq = std::collections::BTreeSet::new();
        for s in req.signers.iter() { uniq.insert(s); }
        if uniq.len() != req.signers.len() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "duplicate signers" }))); }
        if (req.threshold as usize) > req.signers.len() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid threshold" }))); }
        (req.threshold, req.signers.clone())
    };

    let id = uuid::Uuid::new_v4().to_string();
    let signers_json = serde_json::json!(signers);
    if let Err(e) = db::ms_create_proposal(&state.pool, &id, &req.owner, req.amount as i64, threshold as i32, &signers_json).await {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })));
    }
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "ms_propose_withdraw", serde_json::json!({ "proposal_id": id, "amount": req.amount, "threshold": req.threshold, "signers": signers_json })).await;
    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "ms_proposal", "proposal_id": id, "owner": req.owner, "amount": req.amount }).to_string());
    // Notify signers via webhook if available
    if let Ok(contacts) = db::ms_get_contacts_for(&state.pool, &signers).await {
        for (pk, _email, webhook) in contacts.into_iter() {
            if let Some(url) = webhook {
                let payload = serde_json::json!({ "action": "request_signature", "proposal_id": id, "owner": req.owner, "amount": req.amount, "signer": pk });
                let _ = send_webhook(&url, &payload).await;
            }
        }
    }
    (StatusCode::OK, Json(serde_json::json!({ "proposal_id": id })))
}

#[derive(Deserialize)]
pub struct ApproveWithdrawRequest { pub proposal_id: String, pub signer: String, pub nonce: String, pub signature: String }

pub async fn vault_approve_withdraw(State(state): State<AppState>, Json(req): Json<ApproveWithdrawRequest>) -> impl IntoResponse {
    // Verify signer signature on message: approve_withdraw:{proposal_id}:{nonce}
    let message = format!("approve_withdraw:{}:{}", req.proposal_id, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.signer, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.signer).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }

    let prop = match db::ms_get_proposal(&state.pool, &req.proposal_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "proposal not found" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let (owner, amount, threshold, signers_json, status) = prop;
    if status != "pending" { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "proposal not pending" }))); }
    // signer must be in allowed list
    let allowed: Vec<String> = match serde_json::from_value(signers_json.clone()) { Ok(v) => v, Err(_) => vec![] };
    if !allowed.iter().any(|s| s == &req.signer) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "signer not allowed" })));
    }
    match db::ms_insert_approval(&state.pool, &req.proposal_id, &req.signer, &req.signature).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::OK, Json(serde_json::json!({ "ok": true, "duplicate": true })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    let count = match db::ms_count_approvals(&state.pool, &req.proposal_id).await {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };

    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "ms_approval", "proposal_id": req.proposal_id, "signer": req.signer }).to_string());

    if count as i32 >= threshold {
        // Threshold reached. Build a partially signed transaction for co-signing by approvers.
        let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
        let owner_pk = match Pubkey::from_str(&owner) { Ok(p) => p, Err(_) => Pubkey::default() };
        // Collect current approvals as signer set
        let approvals = match db::ms_list_approvals(&state.pool, &req.proposal_id).await { Ok(v) => v, Err(_) => vec![] };
        let mut signer_pubkeys: Vec<Pubkey> = approvals.iter().filter_map(|s| Pubkey::from_str(s).ok()).collect();
        // Ensure current signer is first (authority)
        let authority_pk = Pubkey::from_str(&req.signer).unwrap_or_default();
        signer_pubkeys.retain(|p| *p != authority_pk);
        let params = WithdrawMultisigParams { program_id, owner: owner_pk, authority: authority_pk, amount: amount as u64, other_signers: signer_pubkeys.clone() };
        let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) { Ok(kp) => kp, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))) };
        let raw = match build_partial_withdraw_tx(&state.sol, &payer, &params).await { Ok(b) => b, Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))) };
        let tx_b64 = base64::encode(raw);
        let _ = db::ms_set_status(&state.pool, &req.proposal_id, "approved").await;

        // Notify signers (including authority and others) via webhook with the partial tx
        let signer_strings: Vec<String> = std::iter::once(req.signer.clone()).chain(approvals.into_iter().filter(|s| s != &req.signer)).collect();
        if let Ok(contacts) = db::ms_get_contacts_for(&state.pool, &signer_strings).await {
            for (pk, _email, webhook) in contacts.into_iter() {
                if let Some(url) = webhook {
                    let payload = serde_json::json!({ "action": "partial_tx_ready", "proposal_id": req.proposal_id, "owner": owner, "amount": amount, "tx_base64": tx_b64.clone(), "authority": req.signer });
                    let _ = send_webhook(&url, &payload).await;
                }
            }
        }
        (StatusCode::OK, Json(serde_json::json!({ "ready": true, "transaction_base64": tx_b64, "required_signers": params.other_signers.iter().map(|p| p.to_string()).collect::<Vec<_>>() })))
    } else {
        (StatusCode::OK, Json(serde_json::json!({ "ready": false, "approvals": count, "threshold": threshold })))
    }
}

pub async fn vault_proposal_status(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let prop = match db::ms_get_proposal(&state.pool, &id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "proposal not found" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let (_owner, _amount, threshold, _signers, status) = prop;
    let approvals = match db::ms_count_approvals(&state.pool, &id).await { Ok(c) => c, Err(_) => 0 };
    (StatusCode::OK, Json(serde_json::json!({ "status": status, "approvals": approvals, "threshold": threshold })))
}

// ---------------
// Webhook utility
// ---------------
async fn send_webhook(url: &str, payload: &serde_json::Value) -> Result<(), String> {
    let client = reqwest::Client::new();
    client
        .post(url)
        .json(payload)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    Ok(())
}

// -----------------
// Delegation routes
// -----------------
#[derive(Deserialize)]
pub struct DelegateAddRequest { pub owner: String, pub delegate: String, pub nonce: String, pub signature: String }

pub async fn vault_delegate_add(State(state): State<AppState>, Json(req): Json<DelegateAddRequest>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() || Pubkey::from_str(&req.delegate).is_err() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid pubkey(s)" })));
    }
    // Require owner signature
    let message = format!("delegate_add:{}:{}:{}", req.owner, req.delegate, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {}
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    match db::delegate_add(&state.pool, &req.owner, &req.delegate).await {
        Ok(true) => {
            let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "delegate_add", serde_json::json!({ "delegate": req.delegate })).await;
            (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
        }
        Ok(false) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "duplicate": true }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

#[derive(Deserialize)]
pub struct DelegateRemoveRequest { pub owner: String, pub delegate: String, pub nonce: String, pub signature: String }

pub async fn vault_delegate_remove(State(state): State<AppState>, Json(req): Json<DelegateRemoveRequest>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() || Pubkey::from_str(&req.delegate).is_err() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid pubkey(s)" })));
    }
    // Require owner signature
    let message = format!("delegate_remove:{}:{}:{}", req.owner, req.delegate, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {}
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    match db::delegate_remove(&state.pool, &req.owner, &req.delegate).await {
        Ok(true) => {
            let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "delegate_remove", serde_json::json!({ "delegate": req.delegate })).await;
            (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
        }
        Ok(false) => (StatusCode::OK, Json(serde_json::json!({ "ok": true, "not_found": true }))),
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
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
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
	TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
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
		Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
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
		Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
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

// -----------------
// Yield endpoints
// -----------------
#[derive(Deserialize)]
pub struct YieldOpRequest { pub owner: String, pub amount: u64, pub yield_program: String, pub nonce: String, pub signature: String }

pub async fn vault_yield_deposit(State(state): State<AppState>, Json(req): Json<YieldOpRequest>) -> impl IntoResponse {
    let message = format!("yield_deposit:{}:{}:{}:{}", req.owner, req.amount, req.yield_program, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let yield_program = match Pubkey::from_str(&req.yield_program) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid yield_program" }))) };
    let ix = match build_instruction_yield_deposit(&YieldDepositParams { program_id, owner, amount: req.amount, yield_program }) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({
            "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable
        })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "yield_deposit_request", serde_json::json!({ "amount": req.amount, "yield_program": req.yield_program, "nonce": req.nonce })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

pub async fn vault_yield_withdraw(State(state): State<AppState>, Json(req): Json<YieldOpRequest>) -> impl IntoResponse {
    let message = format!("yield_withdraw:{}:{}:{}:{}", req.owner, req.amount, req.yield_program, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let yield_program = match Pubkey::from_str(&req.yield_program) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid yield_program" }))) };
    let ix = match build_instruction_yield_withdraw(&YieldWithdrawParams { program_id, owner, amount: req.amount, yield_program }) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({
            "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable
        })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "yield_withdraw_request", serde_json::json!({ "amount": req.amount, "yield_program": req.yield_program, "nonce": req.nonce })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

#[derive(Deserialize)]
pub struct YieldCompoundRequest { pub owner: String, pub compounded_amount: u64, pub yield_program: String, pub nonce: String, pub signature: String }

pub async fn vault_compound_yield(State(state): State<AppState>, Json(req): Json<YieldCompoundRequest>) -> impl IntoResponse {
    let message = format!("compound_yield:{}:{}:{}:{}", req.owner, req.compounded_amount, req.yield_program, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" })) ),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => Pubkey::default() };
    let owner = match Pubkey::from_str(&req.owner) { Ok(p) => p, Err(_) => Pubkey::default() };
    let yield_program = match Pubkey::from_str(&req.yield_program) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid yield_program" }))) };
    let ix = match build_instruction_compound_yield(&CompoundYieldParams { program_id, owner, compounded_amount: req.compounded_amount, yield_program }) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({
            "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable
        })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "yield_compound_request", serde_json::json!({ "compounded_amount": req.compounded_amount, "yield_program": req.yield_program, "nonce": req.nonce })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

// -----------------
// Internal: transfer_collateral (admin-only)
// -----------------
#[derive(Deserialize)]
pub struct TransferCollateralRequest { pub from_owner: String, pub to_owner: String, pub amount: u64, pub caller_program: Option<String> }

pub async fn internal_transfer_collateral(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(req): Json<TransferCollateralRequest>,
) -> impl IntoResponse {
    // Admin protection
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }

    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let from_owner = match Pubkey::from_str(&req.from_owner) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid from_owner" }))) };
    let to_owner = match Pubkey::from_str(&req.to_owner) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid to_owner" }))) };
    let mint = match Pubkey::from_str(&state.cfg.usdt_mint) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid usdt mint" }))) };
    let caller_program = if let Some(cp) = &req.caller_program { match Pubkey::from_str(cp) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid caller_program" }))) } } else {
        match Pubkey::from_str(&state.cfg.position_manager_program_id) { Ok(p) => p, Err(_) => Pubkey::default() }
    };

    let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
    let ix = match build_instruction_transfer_collateral(&TransferCollateralParams { program_id, caller_program, from_owner, to_owner, mint, amount: req.amount }) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    ixs.push(ix);

    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) { Ok(kp) => kp, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))) };
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await { Ok(h) => h, Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") }))) };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
    let sig = match send_transaction_with_retries(&state.sol, &tx, 3).await { Ok(s) => s, Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))) };

    let _ = db::insert_audit_log(&state.pool, None, "transfer_collateral", serde_json::json!({ "from_owner": req.from_owner, "to_owner": req.to_owner, "amount": req.amount, "signature": sig.to_string() })).await;
    (StatusCode::OK, Json(serde_json::json!({ "signature": sig.to_string() })))
}

pub async fn vault_yield_status(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    let apys = crate::db::latest_protocol_apys(&state.pool).await.unwrap_or_default();
    let mut best: Option<(&str, f64)> = None;
    let mut list = Vec::new();
    for (protocol, apy) in apys.into_iter() {
        if best.map(|(_, b)| apy > b).unwrap_or(true) {
            best = Some((Box::leak(protocol.clone().into_boxed_str()), apy));
        }
        list.push(serde_json::json!({ "name": protocol, "apy": apy }));
    }
    let (selected, projected_apr) = match best { Some((p, a)) => (Some(p.to_string()), a), None => (None, 0.0) };

    // On-chain yield balances
    let mut yield_deposited = 0u64;
    let mut yield_accrued = 0u64;
    let mut active_program: Option<String> = None;
    if let (Ok(program_id), Ok(owner_pk)) = (Pubkey::from_str(&state.cfg.program_id), Pubkey::from_str(&owner)) {
        if let Ok(snap) = crate::solana_client::fetch_vault_yield_info(&state.sol, &owner_pk, &program_id).await {
            yield_deposited = snap.yield_deposited_balance;
            yield_accrued = snap.yield_accrued_balance;
            if snap.active_yield_program != Pubkey::default() {
                active_program = Some(snap.active_yield_program.to_string());
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({
        "owner": owner,
        "protocols": list,
        "selected": selected,
        "projected_apr": projected_apr,
        "vault": {
            "yield_deposited_balance": yield_deposited,
            "yield_accrued_balance": yield_accrued,
            "active_yield_program": active_program,
        }
    })))
}

// -----------------
// Withdraw policy & whitelist admin
// -----------------

#[derive(Deserialize)]
pub struct AdminWhitelistReq { pub owner: String, pub address: String }

pub async fn admin_withdraw_whitelist_add(State(state): State<AppState>, Json(req): Json<AdminWhitelistReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() || Pubkey::from_str(&req.address).is_err() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid pubkey(s)" })));
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner = Pubkey::from_str(&req.owner).unwrap();
    let address = Pubkey::from_str(&req.address).unwrap();
    let ix = match build_instruction_add_withdraw_whitelist(&program_id, &owner, &address) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({ "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "whitelist_add_requested", serde_json::json!({ "address": req.address })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

pub async fn admin_withdraw_whitelist_remove(State(state): State<AppState>, Json(req): Json<AdminWhitelistReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() || Pubkey::from_str(&req.address).is_err() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid pubkey(s)" })));
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner = Pubkey::from_str(&req.owner).unwrap();
    let address = Pubkey::from_str(&req.address).unwrap();
    let ix = match build_instruction_remove_withdraw_whitelist(&program_id, &owner, &address) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({ "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "whitelist_remove_requested", serde_json::json!({ "address": req.address })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

#[derive(Deserialize)]
pub struct AdminMinDelayReq { pub owner: String, pub seconds: i64 }

pub async fn admin_withdraw_min_delay_set(State(state): State<AppState>, Json(req): Json<AdminMinDelayReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))); }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner = Pubkey::from_str(&req.owner).unwrap();
    let ix = match build_instruction_set_withdraw_min_delay(&program_id, &owner, req.seconds) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({ "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "min_delay_set_requested", serde_json::json!({ "seconds": req.seconds })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

#[derive(Deserialize)]
pub struct AdminRateLimitReq { pub owner: String, pub window_seconds: u32, pub max_amount: u64 }

pub async fn admin_withdraw_rate_limit_set(State(state): State<AppState>, Json(req): Json<AdminRateLimitReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))); }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner = Pubkey::from_str(&req.owner).unwrap();
    let ix = match build_instruction_set_withdraw_rate_limit(&program_id, &owner, req.window_seconds, req.max_amount) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({ "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "rate_limit_set_requested", serde_json::json!({ "window_seconds": req.window_seconds, "max_amount": req.max_amount })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

#[derive(Deserialize)]
pub struct AdminYieldProgramReq { pub yield_program: String }

pub async fn admin_yield_program_add(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(req): Json<AdminYieldProgramReq>,
) -> impl IntoResponse {
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let yield_program = match Pubkey::from_str(&req.yield_program) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid yield program" }))) };
    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) {
        Ok(kp) => kp,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    if !state.cfg.vault_authority_pubkey.is_empty() {
        if let Ok(cfg_va) = Pubkey::from_str(&state.cfg.vault_authority_pubkey) {
            if cfg_va != payer.pubkey() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "payer is not governance signer" })));
            }
        }
    }
    let mut ixs = build_compute_budget_instructions(1_200_000, 1_000);
    let ix = match build_instruction_add_yield_program(&AddYieldProgramParams { program_id, governance: payer.pubkey(), yield_program }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    ixs.push(ix);
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") }))),
    };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
    let sig = match send_transaction_with_retries(&state.sol, &tx, 3).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let signature = sig.to_string();
    let _ = db::insert_audit_log(&state.pool, None, "yield_program_add", serde_json::json!({ "yield_program": req.yield_program, "signature": signature })).await;
    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "yield_program_add", "program": req.yield_program, "signature": signature }).to_string());
    (StatusCode::OK, Json(serde_json::json!({ "signature": signature })))
}

pub async fn admin_yield_program_remove(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(req): Json<AdminYieldProgramReq>,
) -> impl IntoResponse {
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let yield_program = match Pubkey::from_str(&req.yield_program) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid yield program" }))) };
    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) {
        Ok(kp) => kp,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    if !state.cfg.vault_authority_pubkey.is_empty() {
        if let Ok(cfg_va) = Pubkey::from_str(&state.cfg.vault_authority_pubkey) {
            if cfg_va != payer.pubkey() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "payer is not governance signer" })));
            }
        }
    }
    let mut ixs = build_compute_budget_instructions(1_200_000, 1_000);
    let ix = match build_instruction_remove_yield_program(&RemoveYieldProgramParams { program_id, governance: payer.pubkey(), yield_program }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    ixs.push(ix);
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") }))),
    };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
    let sig = match send_transaction_with_retries(&state.sol, &tx, 3).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let signature = sig.to_string();
    let _ = db::insert_audit_log(&state.pool, None, "yield_program_remove", serde_json::json!({ "yield_program": req.yield_program, "signature": signature })).await;
    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "yield_program_remove", "program": req.yield_program, "signature": signature }).to_string());
    (StatusCode::OK, Json(serde_json::json!({ "signature": signature })))
}

#[derive(Deserialize)]
pub struct AdminRiskLevelSetReq { pub risk_level: u8 }

pub async fn admin_risk_level_set(
    State(state): State<AppState>,
    TypedHeader(auth): TypedHeader<Authorization<Bearer>>,
    Json(req): Json<AdminRiskLevelSetReq>,
) -> impl IntoResponse {
    let token = auth.token();
    if state.cfg.admin_jwt_secret.is_empty() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": "admin secret not configured" })));
    }
    if verify_admin_jwt(token, &state.cfg.admin_jwt_secret).is_err() {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "unauthorized" })));
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let payer = match load_deployer_keypair(&state.cfg.deployer_keypair_path) {
        Ok(kp) => kp,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    if !state.cfg.vault_authority_pubkey.is_empty() {
        if let Ok(cfg_va) = Pubkey::from_str(&state.cfg.vault_authority_pubkey) {
            if cfg_va != payer.pubkey() {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "payer is not governance signer" })));
            }
        }
    }
    let mut ixs = build_compute_budget_instructions(1_000_000, 1_000);
    let ix = match build_instruction_set_risk_level(&SetRiskLevelParams { program_id, governance: payer.pubkey(), risk_level: req.risk_level }) {
        Ok(ix) => ix,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    ixs.push(ix);
    let recent_blockhash = match state.sol.rpc.get_latest_blockhash().await {
        Ok(h) => h,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": format!("blockhash: {e}") }))),
    };
    let tx = solana_sdk::transaction::Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
    let sig = match send_transaction_with_retries(&state.sol, &tx, 3).await {
        Ok(s) => s,
        Err(e) => return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({ "error": e.to_string() }))),
    };
    let signature = sig.to_string();
    let _ = db::insert_audit_log(&state.pool, None, "risk_level_set", serde_json::json!({ "risk_level": req.risk_level, "signature": signature })).await;
    let _ = state.notifier.security_tx.send(serde_json::json!({ "kind": "risk_level_set", "risk_level": req.risk_level, "signature": signature }).to_string());
    (StatusCode::OK, Json(serde_json::json!({ "signature": signature })))
}

// -----------------
// 2FA
// -----------------
#[derive(Deserialize)]
pub struct TwoFASetupReq { pub owner: String, pub secret: Option<String> }

#[derive(Deserialize)]
pub struct TwoFAVerifyReq { pub owner: String, pub code: String }

pub async fn twofa_setup(State(state): State<AppState>, Json(req): Json<TwoFASetupReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))); }
    let secret = req.secret.unwrap_or_else(|| "".to_string());
    if secret.is_empty() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "missing secret" }))); }
    if let Err(e) = db::twofa_upsert(&state.pool, &req.owner, &secret, false).await { return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))); }
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

pub async fn twofa_verify(State(state): State<AppState>, Json(req): Json<TwoFAVerifyReq>) -> impl IntoResponse {
    if Pubkey::from_str(&req.owner).is_err() { return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid owner" }))); }
    let row = match db::twofa_get(&state.pool, &req.owner).await { Ok(s) => s, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))) };
    let (secret, _enabled) = match row { Some(r) => r, None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "2fa not set up" }))) };
    // Lightweight TOTP validation using totp-rs (base32 secret assumed)
    match crate::security::verify_totp(&secret, &req.code) {
        true => {
            let _ = db::twofa_upsert(&state.pool, &req.owner, &secret, true).await;
            (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
        }
        false => (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "invalid code" })))
    }
}

// -----------------
// Limits & analytics
// -----------------
pub async fn vault_limits(State(state): State<AppState>, Path(owner): Path<String>) -> impl IntoResponse {
    // Derive current usage from recent transactions (last 24h) as a mirror
    let since = time::OffsetDateTime::now_utc() - time::Duration::hours(24);
    let rows = sqlx::query_as::<_, (i64, String, Option<i64>, String, time::OffsetDateTime)>(
        "SELECT id, signature, amount, kind, created_at FROM transactions WHERE owner = $1 AND kind = 'withdraw' AND created_at >= $2"
    )
    .bind(&owner)
    .bind(since)
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let used: i64 = rows.iter().map(|(_,_,amt,_,_)| amt.unwrap_or(0)).sum();
    (StatusCode::OK, Json(serde_json::json!({ "window_seconds": 86400, "used": used, "limit": null })))
}

pub async fn analytics_tvl_series(State(state): State<AppState>) -> impl IntoResponse {
    // Aggregate transactions by day to build series
    let rows = sqlx::query!(
        "SELECT date_trunc('day', created_at) AS day, SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END) AS delta FROM transactions GROUP BY 1 ORDER BY 1 ASC"
    ).fetch_all(&state.pool).await;
    match rows {
        Ok(rs) => {
            let mut tvl = 0i64;
            let series: Vec<_> = rs.into_iter().map(|r| {
                let d = r.day.unwrap();
                let delta = r.delta.unwrap_or(0);
                tvl += delta;
                serde_json::json!({ "t": d, "tvl": tvl })
            }).collect();
            (StatusCode::OK, Json(serde_json::json!({ "series": series })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

pub async fn analytics_distribution(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query!("SELECT total_balance FROM vaults").fetch_all(&state.pool).await;
    match rows {
        Ok(rs) => {
            let mut buckets = vec![0u64; 10];
            for r in rs.into_iter() {
                let v = (r.total_balance.unwrap_or(0) as u64);
                let idx = std::cmp::min(9, (v as f64).log10().floor().max(0.0) as usize);
                buckets[idx] += 1;
            }
            (StatusCode::OK, Json(serde_json::json!({ "buckets": buckets })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

pub async fn analytics_utilization(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query!("SELECT total_balance, locked_balance FROM vaults").fetch_all(&state.pool).await;
    match rows {
        Ok(rs) => {
            let mut total: i128 = 0;
            let mut locked: i128 = 0;
            for r in rs.into_iter() {
                total += r.total_balance.unwrap_or(0) as i128;
                locked += r.locked_balance.unwrap_or(0) as i128;
            }
            let utilization = if total > 0 { (locked as f64) / (total as f64) } else { 0.0 };
            (StatusCode::OK, Json(serde_json::json!({ "utilization": utilization })))
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
}

// -----------------
// Min-delay withdraw request (on-chain request flow)
// -----------------
#[derive(Deserialize)]
pub struct RequestWithdrawReq { pub owner: String, pub amount: u64, pub nonce: String, pub signature: String }

pub async fn vault_request_withdraw(State(state): State<AppState>, Json(req): Json<RequestWithdrawReq>) -> impl IntoResponse {
    let message = format!("request_withdraw:{}:{}:{}", req.owner, req.amount, req.nonce);
    if let Err(e) = verify_wallet_signature(&req.owner, message.as_bytes(), &req.signature) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": e.to_string() })));
    }
    match db::consume_nonce(&state.pool, &req.nonce, &req.owner).await {
        Ok(true) => {} ,
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() })))
    }
    let program_id = match Pubkey::from_str(&state.cfg.program_id) { Ok(p) => p, Err(_) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid program id" }))) };
    let owner = Pubkey::from_str(&req.owner).unwrap();
    let ix = match build_instruction_request_withdraw(&program_id, &owner, req.amount) { Ok(ix) => ix, Err(e) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": e.to_string() }))) };
    let payload = serde_json::json!({
        "program_id": ix.program_id.to_string(),
        "accounts": ix.accounts.iter().map(|a| serde_json::json!({ "pubkey": a.pubkey.to_string(), "is_signer": a.is_signer, "is_writable": a.is_writable })).collect::<Vec<_>>(),
        "data": base64::encode(ix.data),
    });
    let _ = db::insert_audit_log(&state.pool, Some(&req.owner), "withdraw_request_created", serde_json::json!({ "amount": req.amount })).await;
    (StatusCode::OK, Json(serde_json::json!({ "instruction": payload })))
}

