// End-to-end tests for Database Schema

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::db;
    use sqlx::Row;

    #[tokio::test]
    #[ignore]
    async fn test_vault_accounts_table() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Create vault account
        db::upsert_vault_token_account(&ctx.pool, &owner, &owner)
            .await.expect("Failed to create vault");
        
        db::update_vault_snapshot(&ctx.pool, &owner, 50000, 10000, 0)
            .await.expect("Failed to update snapshot");
        
        // Verify vault exists
        let vault = db::get_vault(&ctx.pool, &owner)
            .await.expect("Failed to get vault");
        
        assert!(vault.is_some());
        let (token_account, balance) = vault.unwrap();
        assert_eq!(token_account, Some(owner.clone()));
        assert_eq!(balance, 50000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_history_table() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Insert transactions with different statuses
        insert_test_transaction(&ctx, &owner, "sig1", 1000, "deposit", "pending")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner, "sig2", 2000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner, "sig3", 500, "withdraw", "failed")
            .await.expect("Failed to insert");
        
        // Query transactions
        let transactions = db::list_transactions(&ctx.pool, &owner, 10, 0)
            .await.expect("Failed to list transactions");
        
        assert_eq!(transactions.len(), 3);
        
        // Verify status tracking
        let rows = sqlx::query(
            "SELECT signature, status FROM transactions WHERE owner = $1 ORDER BY id"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await.expect("Failed to query");
        
        assert_eq!(rows.len(), 3);
        let statuses: Vec<String> = rows.iter()
            .map(|r: &sqlx::postgres::PgRow| r.get::<String, _>("status"))
            .collect();
        assert!(statuses.contains(&"pending".to_string()));
        assert!(statuses.contains(&"confirmed".to_string()));
        assert!(statuses.contains(&"failed".to_string()));
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_balance_snapshots_table() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Insert hourly and daily snapshots
        db::insert_balance_snapshot(&ctx.pool, &owner, 10000, 2000, "hourly")
            .await.expect("Failed to insert hourly snapshot");
        db::insert_balance_snapshot(&ctx.pool, &owner, 15000, 3000, "daily")
            .await.expect("Failed to insert daily snapshot");
        
        // Verify snapshots
        let rows = sqlx::query(
            "SELECT balance, locked_balance, granularity FROM balance_snapshots WHERE owner = $1"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await.expect("Failed to query snapshots");
        
        assert_eq!(rows.len(), 2);
        
        use sqlx::postgres::PgRow;
        let hourly = rows.iter().find(|r: &&PgRow| r.get::<String, _>("granularity") == "hourly").unwrap();
        assert_eq!(hourly.get::<i64, _>("balance"), 10000);
        assert_eq!(hourly.get::<i64, _>("locked_balance"), 2000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_reconciliation_logs_table() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        db::insert_reconciliation_log(
            &ctx.pool,
            &owner,
            Some(&owner),
            50000,
            52000,
            2000,
            1000,
        )
        .await.expect("Failed to insert reconciliation log");
        
        // Verify log
        let rows = sqlx::query(
            "SELECT db_balance, chain_balance, discrepancy FROM reconciliation_logs WHERE vault_owner = $1"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await.expect("Failed to query logs");
        
        assert_eq!(rows.len(), 1);
        let row: &sqlx::postgres::PgRow = &rows[0];
        assert_eq!(row.get::<i64, _>("db_balance"), 50000);
        assert_eq!(row.get::<i64, _>("chain_balance"), 52000);
        assert_eq!(row.get::<i64, _>("discrepancy"), 2000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_audit_trail_table() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        db::insert_audit_log(
            &ctx.pool,
            Some(&owner),
            "test_action",
            serde_json::json!({"key": "value"}),
        )
        .await.expect("Failed to insert audit log");
        
        // Verify audit log
        let rows = sqlx::query(
            "SELECT owner, action, details FROM audit_trail WHERE owner = $1"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await.expect("Failed to query audit trail");
        
        assert_eq!(rows.len(), 1);
        let row: &sqlx::postgres::PgRow = &rows[0];
        assert_eq!(row.get::<String, _>("owner"), owner);
        assert_eq!(row.get::<String, _>("action"), "test_action");
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_status_updates() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        let signature = "test_sig_123";
        
        // Insert pending transaction
        insert_test_transaction(&ctx, &owner, signature, 1000, "deposit", "pending")
            .await.expect("Failed to insert");
        
        // Update to confirmed
        db::update_transaction_status(&ctx.pool, signature, "confirmed")
            .await.expect("Failed to update status");
        
        // Verify status update
        let row: sqlx::postgres::PgRow = sqlx::query(
            "SELECT status FROM transactions WHERE signature = $1"
        )
        .bind(signature)
        .fetch_one(&ctx.pool)
        .await.expect("Failed to query");
        
        assert_eq!(row.get::<String, _>("status"), "confirmed");
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_retry_tracking() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        let signature = "test_sig_retry";
        
        insert_test_transaction(&ctx, &owner, signature, 1000, "deposit", "pending")
            .await.expect("Failed to insert");
        
        // Increment retry count
        db::increment_transaction_retry(&ctx.pool, signature)
            .await.expect("Failed to increment retry");
        db::increment_transaction_retry(&ctx.pool, signature)
            .await.expect("Failed to increment retry");
        
        // Verify retry count
        let row: sqlx::postgres::PgRow = sqlx::query(
            "SELECT retry_count FROM transactions WHERE signature = $1"
        )
        .bind(signature)
        .fetch_one(&ctx.pool)
        .await.expect("Failed to query");
        
        assert_eq!(row.get::<i32, _>("retry_count"), 2);
        
        ctx.cleanup().await;
    }
}
