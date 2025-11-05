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
        build_instruction_withdraw, build_instruction_withdraw_multisig, load_deployer_keypair, DepositParams, WithdrawParams,
        fetch_vault_multisig_config, build_partial_withdraw_tx, WithdrawMultisigParams,
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
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
        Ok(false) => return (StatusCode::OK, Json(serde_json::json!({ "ok": true, "duplicate": true }))),
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
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
        Ok(false) => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({ "error": "invalid or used nonce" }))),
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

