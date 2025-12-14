// End-to-end tests for Balance Tracker functionality

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::vault::VaultManager;
    use cvmsback::db;
    use sqlx::Row;

    #[tokio::test]
    #[ignore]
    async fn test_available_balance_calculation() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        let vm = VaultManager::new(ctx.state.clone());
        
        // Set initial balance
        let total_balance = 100000i64;
        db::update_vault_snapshot(&ctx.pool, &owner, total_balance, 0, 0)
            .await.expect("Failed to set balance");
        
        // No locked balance
        let available = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available, total_balance);
        
        // Lock some balance
        let locked = 30000i64;
        db::increment_locked_balance(&ctx.pool, &owner, locked)
            .await.expect("Failed to lock balance");
        
        let available_after_lock = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available_after_lock, total_balance - locked);
        
        // Unlock some balance
        let unlock_amount = 10000i64;
        db::increment_locked_balance(&ctx.pool, &owner, -unlock_amount)
            .await.expect("Failed to unlock balance");
        
        let available_after_unlock = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available_after_unlock, total_balance - locked + unlock_amount);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_balance_snapshots() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Insert hourly snapshot
        db::insert_balance_snapshot(&ctx.pool, &owner, 10000, 2000, "hourly")
            .await.expect("Failed to insert hourly snapshot");
        
        // Insert daily snapshot
        db::insert_balance_snapshot(&ctx.pool, &owner, 15000, 3000, "daily")
            .await.expect("Failed to insert daily snapshot");
        
        // Verify snapshots exist
        let hourly_snapshots = sqlx::query(
            "SELECT balance, locked_balance FROM balance_snapshots WHERE owner = $1 AND granularity = 'hourly'"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await
        .expect("Failed to query snapshots");
        
        assert_eq!(hourly_snapshots.len(), 1);
        use sqlx::postgres::PgRow;
        let row: &PgRow = &hourly_snapshots[0];
        assert_eq!(row.get::<i64, _>("balance"), 10000);
        assert_eq!(row.get::<i64, _>("locked_balance"), 2000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_reconciliation_logging() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        let db_balance = 50000i64;
        let chain_balance = 52000i64;
        let discrepancy = chain_balance - db_balance;
        let threshold = 1000i64;
        
        // Insert reconciliation log
        db::insert_reconciliation_log(
            &ctx.pool,
            &owner,
            Some(&owner),
            db_balance,
            chain_balance,
            discrepancy,
            threshold,
        )
        .await.expect("Failed to insert reconciliation log");
        
        // Verify log exists
        let logs = sqlx::query(
            "SELECT db_balance, chain_balance, discrepancy FROM reconciliation_logs WHERE vault_owner = $1"
        )
        .bind(&owner)
        .fetch_all(&ctx.pool)
        .await
        .expect("Failed to query reconciliation logs");
        
        assert_eq!(logs.len(), 1);
        let row: &sqlx::postgres::PgRow = &logs[0];
        assert_eq!(row.get::<i64, _>("db_balance"), db_balance);
        assert_eq!(row.get::<i64, _>("chain_balance"), chain_balance);
        assert_eq!(row.get::<i64, _>("discrepancy"), discrepancy);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_low_balance_detection() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        let vm = VaultManager::new(ctx.state.clone());
        
        // Set balance below threshold
        let low_balance = (ctx.state.cfg.low_balance_threshold - 1000) as i64;
        db::update_vault_snapshot(&ctx.pool, &owner, low_balance, 0, 0)
            .await.expect("Failed to set low balance");
        
        let available = vm.available_balance(&owner).await.expect("Failed to get balance");
        assert!(available < ctx.state.cfg.low_balance_threshold);
        
        // The balance monitor task should detect this and send an alert
        // In a real test, you'd verify the notifier received the alert
        
        ctx.cleanup().await;
    }
}
