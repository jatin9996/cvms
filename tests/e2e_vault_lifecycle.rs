// End-to-end tests for complete vault lifecycle
// Initialize → Deposit → [Lock ↔ Unlock] → Withdraw

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::vault::VaultManager;
    use cvmsback::cpi::CPIManager;
    use cvmsback::db;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[tokio::test]
    #[ignore] // Requires database and Solana RPC
    async fn test_vault_lifecycle_complete_flow() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Step 1: Initialize Vault
        let vm = VaultManager::new(ctx.state.clone());
        let init_ix = vm.build_initialize_vault_ix(
            &Pubkey::from_str(&owner).unwrap()
        ).expect("Failed to build initialize instruction");
        
        assert_eq!(init_ix.accounts.len(), 8);
        assert_eq!(init_ix.program_id, Pubkey::from_str(&ctx.state.cfg.program_id).unwrap());
        
        // Create vault in database
        create_test_vault(&ctx, &owner).await.expect("Failed to create test vault");
        
        // Step 2: Deposit
        let deposit_amount = 10000u64;
        let deposit_ix = vm.build_deposit_ix(
            &Pubkey::from_str(&owner).unwrap(),
            deposit_amount
        ).expect("Failed to build deposit instruction");
        
        assert_eq!(deposit_ix.accounts.len(), 6);
        
        // Simulate deposit transaction
        let deposit_sig = generate_test_signature();
        insert_test_transaction(&ctx, &owner, &deposit_sig, deposit_amount as i64, "deposit", "pending")
            .await.expect("Failed to insert deposit transaction");
        
        // Update vault balance
        db::update_vault_snapshot(&ctx.pool, &owner, deposit_amount as i64, deposit_amount as i64, 0)
            .await.expect("Failed to update vault snapshot");
        
        // Step 3: Query Balance
        let _balance = vm.query_balance_by_owner(&owner).await;
        // Note: This will fail without actual on-chain data, but tests the flow
        // In real test, you'd mock the Solana client
        
        let available = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available, deposit_amount as i64);
        
        // Step 4: Lock Collateral (CPI)
        let lock_amount = 5000u64;
        let _cpi_mgr = CPIManager::new(ctx.state.clone());
        
        // Build lock instruction (would normally submit)
        // In real test, verify instruction is built correctly
        
        // Update locked balance
        db::increment_locked_balance(&ctx.pool, &owner, lock_amount as i64)
            .await.expect("Failed to increment locked balance");
        
        let available_after_lock = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available_after_lock, (deposit_amount - lock_amount) as i64);
        
        // Step 5: Unlock Collateral
        db::increment_locked_balance(&ctx.pool, &owner, -(lock_amount as i64))
            .await.expect("Failed to decrement locked balance");
        
        let available_after_unlock = vm.available_balance(&owner).await.expect("Failed to get available balance");
        assert_eq!(available_after_unlock, deposit_amount as i64);
        
        // Step 6: Withdraw
        let withdraw_amount = 3000u64;
        let withdraw_sig = generate_test_signature();
        insert_test_transaction(&ctx, &owner, &withdraw_sig, withdraw_amount as i64, "withdraw", "pending")
            .await.expect("Failed to insert withdraw transaction");
        
        // Update vault balance
        db::update_vault_snapshot(&ctx.pool, &owner, (deposit_amount - withdraw_amount) as i64, 0, withdraw_amount as i64)
            .await.expect("Failed to update vault snapshot");
        
        let final_available = vm.available_balance(&owner).await.expect("Failed to get final balance");
        assert_eq!(final_available, (deposit_amount - withdraw_amount) as i64);
        
        // Step 7: Verify Transaction History
        let transactions = db::list_transactions(&ctx.pool, &owner, 10, 0)
            .await.expect("Failed to list transactions");
        
        assert_eq!(transactions.len(), 2); // deposit and withdraw
        assert!(transactions.iter().any(|(_, sig, _, kind, _)| 
            sig == &deposit_sig && kind == "deposit"));
        assert!(transactions.iter().any(|(_, sig, _, kind, _)| 
            sig == &withdraw_sig && kind == "withdraw"));
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_multiple_deposits_and_withdrawals() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create test vault");
        
        let vm = VaultManager::new(ctx.state.clone());
        
        // Multiple deposits
        let deposits = vec![1000u64, 2000u64, 3000u64];
        let mut total_deposited = 0i64;
        
        for amount in deposits {
            let sig = generate_test_signature();
            insert_test_transaction(&ctx, &owner, &sig, amount as i64, "deposit", "pending")
                .await.expect("Failed to insert deposit");
            total_deposited += amount as i64;
            db::update_vault_snapshot(&ctx.pool, &owner, total_deposited, amount as i64, 0)
                .await.expect("Failed to update snapshot");
        }
        
        // Multiple withdrawals
        let withdrawals = vec![500u64, 1000u64];
        let mut total_withdrawn = 0i64;
        
        for amount in withdrawals {
            let sig = generate_test_signature();
            insert_test_transaction(&ctx, &owner, &sig, amount as i64, "withdraw", "pending")
                .await.expect("Failed to insert withdrawal");
            total_withdrawn += amount as i64;
            db::update_vault_snapshot(&ctx.pool, &owner, total_deposited - total_withdrawn, 0, amount as i64)
                .await.expect("Failed to update snapshot");
        }
        
        let final_balance = vm.available_balance(&owner).await.expect("Failed to get balance");
        assert_eq!(final_balance, total_deposited - total_withdrawn);
        
        // Verify transaction count
        let transactions = db::list_transactions(&ctx.pool, &owner, 100, 0)
            .await.expect("Failed to list transactions");
        assert_eq!(transactions.len(), 5); // 3 deposits + 2 withdrawals
        
        ctx.cleanup().await;
    }
}
