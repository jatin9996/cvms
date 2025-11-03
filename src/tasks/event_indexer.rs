use crate::{api::AppState, db, notify::Notifier};
use solana_client::nonblocking::pubsub_client::PubsubClient;
use solana_client::rpc_config::{RpcTransactionLogsConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_client::rpc_response::RpcTransactionLogsFilter;
use solana_sdk::signature::Signature;
use tracing::{error, info, warn};
use solana_client::rpc_config::RpcTransactionConfig;
use solana_transaction_status::{UiTransactionEncoding, EncodedTransaction, UiMessage, UiRawMessage, UiCompiledInstruction};
use bs58;

pub async fn run_event_indexer(state: AppState, notifier: std::sync::Arc<Notifier>) {
	let ws_url = state.cfg.solana_rpc_url.replace("https://", "wss://").replace("http://", "ws://");
	let program = match solana_sdk::pubkey::Pubkey::from_str(&state.cfg.program_id) {
		Ok(p) => p,
		Err(_) => {
			warn!("program_id not configured; skipping event indexer");
			return;
		}
	};

	loop {
		match PubsubClient::new(&ws_url).await {
			Ok(client) => {
				let filter = RpcTransactionLogsFilter::Mentions(vec![program]);
				let config = RpcTransactionLogsConfig { filter: Some(RpcFilterType::default()), ..Default::default() };
				match client.logs_subscribe(filter, config).await {
					Ok((mut sub, mut stream)) => {
						info!("event indexer subscribed to logs");
						while let Some(update) = stream.next().await {
							match update {
								Ok(logs) => {
									let sig = logs.value.signature.clone();
									let kind = infer_event_kind(&logs.value.logs);
									match parse_and_update(&state, &sig, &kind).await {
										Ok((owner, amount_opt)) => {
											let insert_res = db::insert_transaction(&state.pool, &owner, &sig, amount_opt.map(|a| a as i64), &kind).await;
											if insert_res.is_ok() {
												let payload = serde_json::json!({ "signature": sig, "kind": kind, "owner": owner, "amount": amount_opt });
												let _ = match payload.get("kind").and_then(|k| k.as_str()) {
													Some("deposit") => notifier.deposit_tx.send(payload.to_string()),
													Some("withdraw") => notifier.withdraw_tx.send(payload.to_string()),
													Some("lock") => notifier.lock_tx.send(payload.to_string()),
													Some("unlock") => notifier.unlock_tx.send(payload.to_string()),
													_ => Ok(0),
												};
											}
										}
										Err(e) => warn!("parse/update failed for {sig}: {e}"),
									}
								}
								Err(e) => warn!("log stream error: {e}"),
							}
						}
						let _ = sub.unsubscribe().await;
					}
					Err(e) => {
						warn!("failed to subscribe logs: {e}");
					}
				}
			}
			Err(e) => warn!("pubsub connect error: {e}"),
		}
		tokio::time::sleep(std::time::Duration::from_secs(5)).await;
	}
}

fn infer_event_kind(logs: &Vec<String>) -> String {
	let joined = logs.join("\n").to_lowercase();
	if joined.contains("deposit") { return "deposit".to_string(); }
	if joined.contains("withdraw") { return "withdraw".to_string(); }
	if joined.contains("lock") { return "lock".to_string(); }
	if joined.contains("unlock") { return "unlock".to_string(); }
	"unknown".to_string()
}

async fn parse_and_update(state: &AppState, signature_str: &str, kind: &str) -> Result<(String, Option<u64>), String> {
	let signature = Signature::from_str(signature_str).map_err(|e| format!("bad signature: {e}"))?;
	let cfg = RpcTransactionConfig {
		encoding: Some(UiTransactionEncoding::Json),
		max_supported_transaction_version: Some(0),
		..Default::default()
	};
	let tx_opt = state.sol.rpc.get_transaction(&signature, cfg).await.map_err(|e| format!("get_transaction: {e}"))?;
	let tx = match tx_opt { Some(v) => v, None => return Err("no transaction".to_string()) };
	let (owner, amount_opt) = match &tx.transaction.transaction {
		EncodedTransaction::Json(ui_tx) => {
			match &ui_tx.message {
				UiMessage::Raw(UiRawMessage { account_keys, instructions, .. }) => {
					// find ix to our program
					let prog_index = account_keys.iter().position(|k| k == &state.cfg.program_id).ok_or_else(|| "program id not in account keys".to_string())?;
					let ix = instructions.iter().find(|ix| ix.program_id_index as usize == prog_index).ok_or_else(|| "no program ix".to_string())?;
					let owner_index = ix.accounts.get(0).ok_or_else(|| "no owner account index".to_string())?;
					let owner = account_keys.get(*owner_index as usize).ok_or_else(|| "owner index OOB".to_string())?.clone();
					let data_bytes = bs58::decode(&ix.data).into_vec().map_err(|e| format!("data decode: {e}"))?;
					let amount_opt = if data_bytes.len() >= 9 { // [op | amount u64]
						Some(u64::from_le_bytes(data_bytes[1..9].try_into().unwrap()))
					} else { None };
					(owner, amount_opt)
				}
				_ => ("unknown".to_string(), None),
			}
		}
		_ => ("unknown".to_string(), None),
	};

	// Update snapshot
	let (token_account_opt, prev_balance) = db::get_vault(&state.pool, &owner).await.map_err(|e| format!("db get_vault: {e}"))?.unwrap_or((None, 0));
	let (dep_delta, wd_delta) = match kind {
		"deposit" => (amount_opt.unwrap_or(0) as i64, 0i64),
		"withdraw" => (0i64, amount_opt.unwrap_or(0) as i64),
		_ => (0, 0),
	};
	let new_balance = if let Some(ref token_acc) = token_account_opt {
		if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(token_acc) {
			crate::solana_client::get_token_balance(&state.sol, &pk).await.unwrap_or((prev_balance as u64)) as i64
		} else { prev_balance + dep_delta - wd_delta }
	} else { prev_balance + dep_delta - wd_delta };
	let _ = db::update_vault_snapshot(&state.pool, &owner, new_balance, dep_delta, wd_delta).await;
	let _ = state.notifier.vault_balance_tx.send(serde_json::json!({ "owner": owner, "balance": new_balance }).to_string());
	Ok((owner, amount_opt))
}


