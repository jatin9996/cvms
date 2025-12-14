use crate::{api::AppState, db, notify::Notifier, solana_client::get_token_balance};
use std::str::FromStr;
use tracing::{info, warn};

pub async fn run_balance_monitor(
    state: AppState,
    notifier: std::sync::Arc<Notifier>,
    interval_seconds: u64,
) {
    let mut last_balances: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    
    loop {
        match db::list_vaults(&state.pool).await {
            Ok(vaults) => {
                for (owner, token_account_opt, _db_balance) in vaults {
                    if let Some(token_account) = token_account_opt {
                        if let Ok(pk) = solana_sdk::pubkey::Pubkey::from_str(&token_account) {
                            match get_token_balance(&state.sol, &pk).await {
                                Ok(chain_balance) => {
                                    // Detect balance changes
                                    if let Some(last_balance) = last_balances.get(&owner) {
                                        if *last_balance != chain_balance {
                                            let delta = chain_balance as i64 - *last_balance as i64;
                                            info!(
                                                owner = %owner,
                                                old_balance = *last_balance,
                                                new_balance = chain_balance,
                                                delta = delta,
                                                "Balance change detected"
                                            );
                                            
                                            // Notify via WebSocket
                                            let _ = notifier.vault_balance_tx.send(
                                                serde_json::json!({
                                                    "owner": owner,
                                                    "balance": chain_balance,
                                                    "previous_balance": *last_balance,
                                                    "delta": delta,
                                                    "timestamp": time::OffsetDateTime::now_utc(),
                                                })
                                                .to_string(),
                                            );
                                        }
                                    }
                                    last_balances.insert(owner.clone(), chain_balance);
                                    
                                    // Proactive low balance alert
                                    let available = db::get_locked_balance(&state.pool, &owner)
                                        .await
                                        .map(|locked| chain_balance as i64 - locked)
                                        .unwrap_or(chain_balance as i64);
                                    
                                    if available < state.cfg.low_balance_threshold && available > 0 {
                                        let _ = notifier.security_tx.send(
                                            serde_json::json!({
                                                "type": "low_balance_alert",
                                                "owner": owner,
                                                "available_balance": available,
                                                "threshold": state.cfg.low_balance_threshold,
                                                "timestamp": time::OffsetDateTime::now_utc(),
                                            })
                                            .to_string(),
                                        );
                                    }
                                }
                                Err(e) => {
                                    warn!(owner = %owner, error = %e, "Failed to get balance");
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to list vaults for balance monitoring");
            }
        }
        
        tokio::time::sleep(std::time::Duration::from_secs(interval_seconds)).await;
    }
}
