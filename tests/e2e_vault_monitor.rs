// End-to-end tests for Vault Monitor functionality

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use sqlx::Row;

    #[tokio::test]
    #[ignore]
    async fn test_tvl_calculation() {
        let ctx = TestContext::new().await;
        
        // Create multiple vaults with deposits/withdrawals
        let owner1 = TestContext::generate_test_owner();
        let owner2 = TestContext::generate_test_owner();
        let owner3 = TestContext::generate_test_owner();
        
        // Deposits
        insert_test_transaction(&ctx, &owner1, "sig1", 10000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner2, "sig2", 20000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner3, "sig3", 15000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        
        // Withdrawals
        insert_test_transaction(&ctx, &owner1, "sig4", 3000, "withdraw", "confirmed")
            .await.expect("Failed to insert");
        
        // Calculate TVL
        let row = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END), 0) AS tvl FROM transactions"
        )
        .fetch_one(&ctx.pool)
        .await.expect("Failed to calculate TVL");
        
        // Expected: 10000 + 20000 + 15000 - 3000 = 42000
        assert_eq!(row, 42000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_unusual_activity_detection() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Simulate unusual activity: 15 transactions in quick succession
        for i in 0..15 {
            let sig = format!("sig{}", i);
            insert_test_transaction(&ctx, &owner, &sig, 100, "withdraw", "confirmed")
                .await.expect("Failed to insert");
        }
        
        // Query for unusual activity (> 10 transactions in last minute)
        let rows = sqlx::query(
            "SELECT owner, COUNT(*)::BIGINT as cnt FROM transactions 
             WHERE created_at > NOW() - INTERVAL '60 seconds' 
             GROUP BY owner HAVING COUNT(*) > 10"
        )
        .fetch_all(&ctx.pool)
        .await.expect("Failed to query");
        
        assert_eq!(rows.len(), 1);
        let row: &sqlx::postgres::PgRow = &rows[0];
        assert_eq!(row.get::<String, _>("owner"), owner);
        assert!(row.get::<i64, _>("cnt") > 10);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_vault_analytics() {
        let ctx = TestContext::new().await;
        
        // Create multiple vaults
        let owners: Vec<String> = (0..5).map(|_| TestContext::generate_test_owner()).collect();
        
        for owner in &owners {
            create_test_vault(&ctx, owner).await.expect("Failed to create vault");
        }
        
        // Insert some transactions
        for (i, owner) in owners.iter().enumerate() {
            insert_test_transaction(&ctx, owner, &format!("sig{}", i), 1000 * (i as i64 + 1), "deposit", "confirmed")
                .await.expect("Failed to insert");
        }
        
        // Query vault count
        let vault_count: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM vaults")
            .fetch_one(&ctx.pool)
            .await.expect("Failed to query");
        
        assert_eq!(vault_count, 5);
        
        // Query user count
        let user_count: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT owner)::BIGINT FROM transactions")
            .fetch_one(&ctx.pool)
            .await.expect("Failed to query");
        
        assert_eq!(user_count, 5);
        
        // Query 24h volume
        let volume_24h: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(ABS(amount)), 0)::BIGINT FROM transactions WHERE created_at > NOW() - INTERVAL '24 hours'"
        )
        .fetch_one(&ctx.pool)
        .await.expect("Failed to query");
        
        assert!(volume_24h > 0);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_active_vaults_tracking() {
        let ctx = TestContext::new().await;
        
        // Create vaults with different statuses
        let owner1 = TestContext::generate_test_owner();
        let owner2 = TestContext::generate_test_owner();
        
        create_test_vault(&ctx, &owner1).await.expect("Failed to create");
        create_test_vault(&ctx, &owner2).await.expect("Failed to create");
        
        // Update status (if status column exists)
        sqlx::query("UPDATE vaults SET status = 'active' WHERE owner = $1")
            .bind(&owner1)
            .execute(&ctx.pool)
            .await.expect("Failed to update");
        
        sqlx::query("UPDATE vaults SET status = 'inactive' WHERE owner = $2")
            .bind(&owner2)
            .execute(&ctx.pool)
            .await.expect("Failed to update");
        
        // Count active vaults
        let active_count: i64 = sqlx::query_scalar("SELECT COUNT(*)::BIGINT FROM vaults WHERE status = 'active'")
            .fetch_one(&ctx.pool)
            .await.expect("Failed to query");
        
        assert_eq!(active_count, 1);
        
        ctx.cleanup().await;
    }
}
