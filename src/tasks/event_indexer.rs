use crate::{api::AppState, db, notify::Notifier};
use bs58;
use futures::StreamExt;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::{RpcTransactionConfig, RpcTransactionLogsConfig};
use solana_client::rpc_response::RpcTransactionLogsFilter;
use solana_sdk::{pubkey::Pubkey, signature::Signature};
use solana_transaction_status::{EncodedTransaction, UiMessage, UiRawMessage, UiTransactionEncoding};
use std::{str::FromStr, sync::Arc};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

pub async fn run_event_indexer(state: AppState, notifier: Arc<Notifier>) {
    let program = match Pubkey::from_str(&state.cfg.program_id) {
        Ok(p) => p,
        Err(_) => {
            warn!("program_id not configured; skipping event indexer");
            return;
        }
    };
    let ws_url_base = state.cfg.solana_rpc_url.clone();

    loop {
        let ws_url = ws_url_base
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        match PubsubClient::new(&ws_url).await {
            Ok(client) => {
                let filter = RpcTransactionLogsFilter::Mentions(vec![program]);
                let config = RpcTransactionLogsConfig { commitment: None };
                match client.logs_subscribe(filter, config).await {
                    Ok((mut stream, unsubscribe)) => {
                        info!("event indexer subscribed to logs");
                        while let Some(logs) = stream.next().await {
                            let sig = logs.value.signature.clone();
                            let kind = infer_event_kind(&logs.value.logs);
                            match parse_and_update(&state, &sig, &kind).await {
                                Ok((owner, amount_opt)) => {
                                    match db::insert_transaction(
                                        &state.pool,
                                        &owner,
                                        &sig,
                                        amount_opt.map(|a| a as i64),
                                        &kind,
                                    )
                                    .await
                                    {
                                        Ok(_) => {
                                            let payload = serde_json::json!({
                                                "signature": sig,
                                                "kind": kind,
                                                "owner": owner,
                                                "amount": amount_opt
                                            });
                                            let message = payload.to_string();
                                            let _ = match payload.get("kind").and_then(|k| k.as_str()) {
                                                Some("deposit") => notifier.deposit_tx.send(message.clone()),
                                                Some("withdraw") => notifier.withdraw_tx.send(message.clone()),
                                                Some("lock") => notifier.lock_tx.send(message.clone()),
                                                Some("unlock") => notifier.unlock_tx.send(message),
                                                _ => Ok(0),
                                            };
                                        }
                                        Err(e) => warn!("failed to persist transaction {sig}: {e}"),
                                    }
                                }
                                Err(e) => warn!("parse/update failed for {sig}: {e}"),
                            }
                        }
                        unsubscribe().await;
                    }
                    Err(e) => warn!("failed to subscribe logs: {e}"),
                }
            }
            Err(e) => warn!("pubsub connect error: {e}"),
        }
        sleep(Duration::from_secs(5)).await;
    }
}

fn infer_event_kind(logs: &[String]) -> String {
    let joined = logs.join("\n").to_lowercase();
    if joined.contains("deposit") {
        return "deposit".to_string();
    }
    if joined.contains("withdraw") {
        return "withdraw".to_string();
    }
    if joined.contains("lock") {
        return "lock".to_string();
    }
    if joined.contains("unlock") {
        return "unlock".to_string();
    }
    "unknown".to_string()
}

async fn parse_and_update(state: &AppState, signature_str: &str, kind: &str) -> Result<(String, Option<u64>), String> {
    let signature = Signature::from_str(signature_str).map_err(|e| format!("bad signature: {e}"))?;
    let cfg = RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::Json),
        max_supported_transaction_version: Some(0),
        ..Default::default()
    };
    let tx = state
        .sol
        .rpc
        .get_transaction_with_config(&signature, cfg)
        .await
        .map_err(|e| format!("get_transaction: {e}"))?;
    let (owner, amount_opt) = match &tx.transaction.transaction {
        EncodedTransaction::Json(ui_tx) => match &ui_tx.message {
            UiMessage::Raw(UiRawMessage { account_keys, instructions, .. }) => {
                let prog_index = account_keys
                    .iter()
                    .position(|k| k == &state.cfg.program_id)
                    .ok_or_else(|| "program id not in account keys".to_string())?;
                let ix = instructions
                    .iter()
                    .find(|ix| ix.program_id_index as usize == prog_index)
                    .ok_or_else(|| "no program ix".to_string())?;
                let owner_index = ix
                    .accounts
                    .get(0)
                    .ok_or_else(|| "no owner account index".to_string())?;
                let owner = account_keys
                    .get(*owner_index as usize)
                    .ok_or_else(|| "owner index OOB".to_string())?
                    .clone();
                let data_bytes =
                    bs58::decode(&ix.data).into_vec().map_err(|e| format!("data decode: {e}"))?;
                let amount_opt = if data_bytes.len() >= 9 {
                    Some(u64::from_le_bytes(data_bytes[1..9].try_into().unwrap()))
                } else {
                    None
                };
                (owner, amount_opt)
            }
            _ => ("unknown".to_string(), None),
        },
        _ => ("unknown".to_string(), None),
    };

    let (token_account_opt, prev_balance) =
        db::get_vault(&state.pool, &owner).await.map_err(|e| format!("db get_vault: {e}"))?.unwrap_or((None, 0));
    let (dep_delta, wd_delta) = match kind {
        "deposit" => (amount_opt.unwrap_or(0) as i64, 0i64),
        "withdraw" => (0i64, amount_opt.unwrap_or(0) as i64),
        _ => (0, 0),
    };
    let fallback_balance = prev_balance + dep_delta - wd_delta;
    let new_balance = if let Some(ref token_acc) = token_account_opt {
        if let Ok(pk) = Pubkey::from_str(token_acc) {
            match crate::solana_client::get_token_balance(&state.sol, &pk).await {
                Ok(amount) => amount as i64,
                Err(e) => {
                    warn!("get_token_balance failed for {token_acc}: {e}");
                    fallback_balance
                }
            }
        } else {
            fallback_balance
        }
    } else {
        fallback_balance
    };

    let _ = db::update_vault_snapshot(&state.pool, &owner, new_balance, dep_delta, wd_delta).await;
    let _ = state
        .notifier
        .vault_balance_tx
        .send(serde_json::json!({ "owner": owner, "balance": new_balance }).to_string());
    Ok((owner, amount_opt))
}
