use crate::{api::AppState, db, notify::Notifier, solana_client::get_token_balance};
use tracing::{info, warn};

pub async fn run_reconciliation(state: AppState, notifier: std::sync::Arc<Notifier>) {
	let threshold = state.cfg.reconciliation_threshold;
	loop {
		match db::list_vaults(&state.pool).await {
			Ok(vaults) => {
				for (owner, token_account_opt, db_balance) in vaults {
					if let Some(token_account) = token_account_opt.clone() {
						if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(&token_account) {
							match get_token_balance(&state.sol, &pk).await {
								Ok(chain_balance) => {
									let discrepancy = (chain_balance as i64) - db_balance;
									if discrepancy.abs() > threshold {
										let _ = db::insert_reconciliation_log(
											&state.pool,
											&owner,
											Some(&token_account),
											db_balance,
											chain_balance as i64,
											discrepancy,
											threshold,
										).await;
										let payload = serde_json::json!({
											"owner": owner,
											"token_account": token_account,
											"db_balance": db_balance,
											"chain_balance": chain_balance,
											"discrepancy": discrepancy,
											"threshold": threshold,
										});
										let _ = notifier.vault_balance_tx.send(payload.to_string());
									}
								}
								Err(e) => warn!("recon get balance error: {e}"),
							}
						}
					}
				}
			}
			Err(e) => warn!("recon list_vaults error: {e}"),
		}
		tokio::time::sleep(std::time::Duration::from_secs(60)).await;
	}
}


