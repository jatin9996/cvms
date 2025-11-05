use crate::{api::AppState, notify::Notifier};
use tracing::warn;

pub async fn run_monitor(state: AppState, notifier: std::sync::Arc<Notifier>) {
	let low_threshold = state.cfg.low_balance_threshold;
	loop {
		// TVL compute and broadcast
		let tvl_row = sqlx::query_scalar!(
			"SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END), 0) AS tvl FROM transactions"
		).fetch_one(&state.pool).await;
		if let Ok(tvl) = tvl_row {
			let _ = notifier.tvl_tx.send(serde_json::json!({ "tvl": tvl }).to_string());
		}

		// Analytics: vault count, users, 24h volume
		let vault_count = sqlx::query_scalar!("SELECT COUNT(*)::BIGINT FROM vaults").fetch_one(&state.pool).await.unwrap_or(0);
		let user_count = sqlx::query_scalar!("SELECT COUNT(DISTINCT owner)::BIGINT FROM transactions").fetch_one(&state.pool).await.unwrap_or(0);
		let volume_24h = sqlx::query_scalar!(
			"SELECT COALESCE(SUM(ABS(amount)), 0)::BIGINT FROM transactions WHERE created_at > NOW() - INTERVAL '24 hours'"
		).fetch_one(&state.pool).await.unwrap_or(0);
		let _ = notifier.analytics_tx.send(serde_json::json!({
			"vaults": vault_count,
			"users": user_count,
			"volume_24h": volume_24h,
		}).to_string());

		// Low-balance alerts
		if low_threshold > 0 {
			let rows = sqlx::query!("SELECT owner, total_balance, locked_balance FROM vaults")
				.fetch_all(&state.pool)
				.await;
			if let Ok(rows) = rows {
				for r in rows {
					let available = r.total_balance - r.locked_balance;
					if available < low_threshold {
						let _ = notifier.security_tx.send(serde_json::json!({
							"owner": r.owner,
							"type": "low_balance",
							"available": available,
							"threshold": low_threshold,
						}).to_string());
					}
				}
			}
		}

		// Unusual activity: > N tx in last minute
		let rows = sqlx::query!(
			"SELECT owner, COUNT(*)::BIGINT as cnt FROM transactions WHERE created_at > NOW() - INTERVAL '60 seconds' GROUP BY owner HAVING COUNT(*) > 10"
		).fetch_all(&state.pool).await;
		if let Ok(rows) = rows {
			for r in rows {
				let _ = notifier.security_tx.send(serde_json::json!({
					"owner": r.owner,
					"type": "unusual_activity",
					"count": r.cnt.unwrap_or(0),
				}).to_string());
			}
		}

		tokio::time::sleep(std::time::Duration::from_secs(30)).await;
	}
}


