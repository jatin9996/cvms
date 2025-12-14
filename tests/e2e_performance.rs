// Performance tests for the backend

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::vault::VaultManager;
    use cvmsback::db;
    use std::time::Instant;

    #[tokio::test]
    #[ignore]
    async fn test_balance_query_performance() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        db::update_vault_snapshot(&ctx.pool, &owner, 50000, 10000, 0)
            .await.expect("Failed to set balance");
        
        let vm = VaultManager::new(ctx.state.clone());
        
        // Measure balance query time
        let start = Instant::now();
        let _balance = vm.available_balance(&owner).await.expect("Failed to get balance");
        let duration = start.elapsed();
        
        // Should be < 50ms as per requirements
        assert!(duration.as_millis() < 50, "Balance query took {}ms, expected < 50ms", duration.as_millis());
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_history_query_performance() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Insert 100 transactions
        for i in 0..100 {
            let sig = format!("sig{}", i);
            insert_test_transaction(&ctx, &owner, &sig, 1000, "deposit", "confirmed")
                .await.expect("Failed to insert");
        }
        
        // Measure query time
        let start = Instant::now();
        let _transactions = db::list_transactions(&ctx.pool, &owner, 50, 0)
            .await.expect("Failed to list transactions");
        let duration = start.elapsed();
        
        // Should be reasonably fast even with 100 transactions
        assert!(duration.as_millis() < 100, "Transaction query took {}ms", duration.as_millis());
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_concurrent_balance_queries() {
        let ctx = TestContext::new().await;
        let vm = VaultManager::new(ctx.state.clone());
        
        // Create multiple vaults
        let owners: Vec<String> = (0..10).map(|_| TestContext::generate_test_owner()).collect();
        
        for owner in &owners {
            create_test_vault(&ctx, owner).await.expect("Failed to create vault");
            db::update_vault_snapshot(&ctx.pool, owner, 50000, 0, 0)
                .await.expect("Failed to set balance");
        }
        
        // Query all balances concurrently
        let start = Instant::now();
        let handles: Vec<_> = owners.iter()
            .map(|owner: &String| {
                let vm = vm.clone();
                let owner = owner.clone();
                tokio::spawn(async move {
                    vm.available_balance(&owner).await
                })
            })
            .collect();
        
        for handle in handles {
            let _: Result<i64, _> = handle.await.expect("Task failed");
        }
        let duration = start.elapsed();
        
        // 10 concurrent queries should complete quickly
        assert!(duration.as_millis() < 500, "10 concurrent queries took {}ms", duration.as_millis());
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_large_scale_vault_operations() {
        let ctx = TestContext::new().await;
        let vm = VaultManager::new(ctx.state.clone());
        
        // Simulate 1000 vaults (scaling test)
        let num_vaults = 1000;
        let owners: Vec<String> = (0..num_vaults)
            .map(|_| TestContext::generate_test_owner())
            .collect();
        
        // Create all vaults
        let start = Instant::now();
        for owner in &owners {
            create_test_vault(&ctx, owner).await.expect("Failed to create vault");
        }
        let create_duration = start.elapsed();
        
        println!("Created {} vaults in {:?}", num_vaults, create_duration);
        
        // Query balances for all vaults
        let start = Instant::now();
        for owner in &owners[..100] { // Sample first 100
            let _ = vm.available_balance(owner).await;
        }
        let query_duration = start.elapsed();
        
        println!("Queried 100 vaults in {:?}", query_duration);
        assert!(query_duration.as_secs() < 10, "Querying 100 vaults should be fast");
        
        ctx.cleanup().await;
    }
}
