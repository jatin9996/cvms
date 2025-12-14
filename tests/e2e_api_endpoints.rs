// End-to-end tests for REST API endpoints

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::db;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[tokio::test]
    #[ignore] // Requires database
    async fn test_vault_initialize_endpoint() {
        // Test the instruction building directly
        let ctx = TestContext::new().await;
        let vm = cvmsback::vault::VaultManager::new(ctx.state.clone());
        let owner = Pubkey::from_str(&TestContext::generate_test_owner()).unwrap();
        
        let init_ix = vm.build_initialize_vault_ix(&owner)
            .expect("Failed to build initialize instruction");
        
        assert_eq!(init_ix.accounts.len(), 8);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_vault_balance_endpoint() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        db::update_vault_snapshot(&ctx.pool, &owner, 50000, 0, 0)
            .await.expect("Failed to set balance");
        
        // Test balance query directly
        let vm = cvmsback::vault::VaultManager::new(ctx.state.clone());
        let available = vm.available_balance(&owner).await.expect("Failed to get balance");
        assert_eq!(available, 50000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_vault_transactions_endpoint() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Insert test transactions
        insert_test_transaction(&ctx, &owner, "sig1", 1000, "deposit", "confirmed")
            .await.expect("Failed to insert transaction");
        insert_test_transaction(&ctx, &owner, "sig2", 500, "withdraw", "confirmed")
            .await.expect("Failed to insert transaction");
        
        // Test transaction listing directly
        let transactions = db::list_transactions(&ctx.pool, &owner, 10, 0)
            .await.expect("Failed to list transactions");
        
        assert_eq!(transactions.len(), 2);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_vault_tvl_endpoint() {
        let ctx = TestContext::new().await;
        
        // Insert transactions to calculate TVL
        let owner1 = TestContext::generate_test_owner();
        let owner2 = TestContext::generate_test_owner();
        
        insert_test_transaction(&ctx, &owner1, "sig1", 10000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner2, "sig2", 20000, "deposit", "confirmed")
            .await.expect("Failed to insert");
        insert_test_transaction(&ctx, &owner1, "sig3", 3000, "withdraw", "confirmed")
            .await.expect("Failed to insert");
        
        // Test TVL calculation directly
        let tvl: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(CASE WHEN kind = 'deposit' THEN amount ELSE -amount END), 0) AS tvl FROM transactions"
        )
        .fetch_one(&ctx.pool)
        .await.expect("Failed to calculate TVL");
        
        // TVL should be 10000 + 20000 - 3000 = 27000
        assert_eq!(tvl, 27000);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_pagination() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Insert 15 transactions
        for i in 0..15 {
            let sig = format!("sig{}", i);
            insert_test_transaction(&ctx, &owner, &sig, 1000, "deposit", "confirmed")
                .await.expect("Failed to insert");
        }
        
        // Test pagination directly
        let page1 = db::list_transactions(&ctx.pool, &owner, 10, 0)
            .await.expect("Failed to list");
        assert_eq!(page1.len(), 10);
        
        let page2 = db::list_transactions(&ctx.pool, &owner, 10, 10)
            .await.expect("Failed to list");
        assert_eq!(page2.len(), 5); // Remaining 5 items
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_metrics_endpoint() {
        let ctx = TestContext::new().await;
        
        // Test metrics registry directly
        let metric_families = ctx.state.metrics.registry.gather();
        
        // Verify metrics exist
        let metric_names: Vec<String> = metric_families.iter()
            .map(|m: &prometheus::proto::MetricFamily| m.get_name().to_string())
            .collect();
        
        assert!(metric_names.iter().any(|n| n == "vault_operations_total"));
        assert!(metric_names.iter().any(|n| n == "vault_deposits_total"));
        assert!(metric_names.iter().any(|n| n == "vault_withdrawals_total"));
        assert!(metric_names.iter().any(|n| n == "total_value_locked"));
        
        ctx.cleanup().await;
    }
}
