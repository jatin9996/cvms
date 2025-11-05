use crate::{api::AppState, db, notify::Notifier};
use tracing::warn;

pub async fn run_timelock_cron(state: AppState, notifier: std::sync::Arc<Notifier>) {
    loop {
        // Notify items due within next 5 minutes
        if let Ok(rows) = db::timelock_due_within(&state.pool, 300).await {
            for (id, owner, amount, unlock_at) in rows.iter() {
                let _ = notifier.timelock_tx.send(serde_json::json!({
                    "owner": owner,
                    "amount": amount,
                    "unlock_at": unlock_at,
                    "kind": "due_soon",
                }).to_string());
            }
        }
        // Mark available ones and notify
        if let Ok(rows) = db::timelock_due_within(&state.pool, 0).await {
            for (id, owner, amount, unlock_at) in rows.iter() {
                if let Err(e) = db::timelock_mark_status(&state.pool, *id, "available").await {
                    warn!("timelock_mark_status error: {e}");
                } else {
                    let _ = notifier.timelock_tx.send(serde_json::json!({
                        "owner": owner,
                        "amount": amount,
                        "unlock_at": unlock_at,
                        "kind": "available",
                    }).to_string());
                // Optional: auto-execute withdraw transaction (best-effort)
                // This requires an admin payer; omitted here for safety
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}


