// End-to-end tests for Cross-Program Integration (CPIManager)

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::cpi::CPIManager;
    use cvmsback::solana_client::{build_instruction_pm_lock, build_instruction_pm_unlock};
    use cvmsback::db;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[tokio::test]
    #[ignore]
    async fn test_lock_collateral_flow() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        let owner_pk = Pubkey::from_str(&owner).unwrap();
        
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Set initial balance
        db::update_vault_snapshot(&ctx.pool, &owner, 100000, 0, 0)
            .await.expect("Failed to set balance");
        
        let _cpi_mgr = CPIManager::new(ctx.state.clone());
        let lock_amount = 25000u64;
        
        // Build lock instruction
        let pm_pid = Pubkey::from_str(&ctx.state.cfg.position_manager_program_id).unwrap();
        let vault_pid = Pubkey::from_str(&ctx.state.cfg.program_id).unwrap();
        
        let lock_ix = build_instruction_pm_lock(&pm_pid, &vault_pid, &owner_pk, lock_amount)
            .expect("Failed to build lock instruction");
        
        assert_eq!(lock_ix.program_id, pm_pid);
        assert_eq!(lock_ix.accounts.len(), 2);
        
        // Simulate lock (update locked balance)
        db::increment_locked_balance(&ctx.pool, &owner, lock_amount as i64)
            .await.expect("Failed to lock balance");
        
        // Verify locked balance
        let locked = db::get_locked_balance(&ctx.pool, &owner)
            .await.expect("Failed to get locked balance");
        assert_eq!(locked, lock_amount as i64);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_unlock_collateral_flow() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        let owner_pk = Pubkey::from_str(&owner).unwrap();
        
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Set initial locked balance
        db::update_vault_snapshot(&ctx.pool, &owner, 100000, 0, 0)
            .await.expect("Failed to set balance");
        db::increment_locked_balance(&ctx.pool, &owner, 30000)
            .await.expect("Failed to set locked balance");
        
        let _cpi_mgr = CPIManager::new(ctx.state.clone());
        let unlock_amount = 15000u64;
        
        // Build unlock instruction
        let pm_pid = Pubkey::from_str(&ctx.state.cfg.position_manager_program_id).unwrap();
        let vault_pid = Pubkey::from_str(&ctx.state.cfg.program_id).unwrap();
        
        let unlock_ix = build_instruction_pm_unlock(&pm_pid, &vault_pid, &owner_pk, unlock_amount)
            .expect("Failed to build unlock instruction");
        
        assert_eq!(unlock_ix.program_id, pm_pid);
        
        // Simulate unlock
        db::increment_locked_balance(&ctx.pool, &owner, -(unlock_amount as i64))
            .await.expect("Failed to unlock balance");
        
        // Verify locked balance decreased
        let locked = db::get_locked_balance(&ctx.pool, &owner)
            .await.expect("Failed to get locked balance");
        assert_eq!(locked, 15000); // 30000 - 15000
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_lock_unlock_consistency() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        db::update_vault_snapshot(&ctx.pool, &owner, 100000, 0, 0)
            .await.expect("Failed to set balance");
        
        // Lock
        db::increment_locked_balance(&ctx.pool, &owner, 20000)
            .await.expect("Failed to lock");
        
        // Unlock
        db::increment_locked_balance(&ctx.pool, &owner, -20000)
            .await.expect("Failed to unlock");
        
        // Verify balance is back to zero locked
        let locked = db::get_locked_balance(&ctx.pool, &owner)
            .await.expect("Failed to get locked balance");
        assert_eq!(locked, 0);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_multiple_locks_and_unlocks() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        db::update_vault_snapshot(&ctx.pool, &owner, 100000, 0, 0)
            .await.expect("Failed to set balance");
        
        // Multiple locks
        let locks = vec![10000i64, 15000i64, 5000i64];
        let mut total_locked = 0i64;
        
        for amount in locks {
            db::increment_locked_balance(&ctx.pool, &owner, amount)
                .await.expect("Failed to lock");
            total_locked += amount;
        }
        
        assert_eq!(
            db::get_locked_balance(&ctx.pool, &owner).await.unwrap(),
            total_locked
        );
        
        // Multiple unlocks
        let unlocks = vec![5000i64, 10000i64];
        for amount in unlocks {
            db::increment_locked_balance(&ctx.pool, &owner, -amount)
                .await.expect("Failed to unlock");
            total_locked -= amount;
        }
        
        assert_eq!(
            db::get_locked_balance(&ctx.pool, &owner).await.unwrap(),
            total_locked
        );
        
        ctx.cleanup().await;
    }

    #[test]
    fn test_cpi_instruction_opcodes() {
        let pm_program = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 10000u64;
        
        let lock_ix = build_instruction_pm_lock(&pm_program, &vault_program, &owner, amount)
            .expect("Failed to build lock instruction");
        let unlock_ix = build_instruction_pm_unlock(&pm_program, &vault_program, &owner, amount)
            .expect("Failed to build unlock instruction");
        
        // Verify opcodes
        assert_eq!(lock_ix.data[0], 10); // lock opcode
        assert_eq!(unlock_ix.data[0], 11); // unlock opcode
        
        // Verify amounts
        let lock_amount = u64::from_le_bytes(lock_ix.data[1..9].try_into().unwrap());
        let unlock_amount = u64::from_le_bytes(unlock_ix.data[1..9].try_into().unwrap());
        assert_eq!(lock_amount, amount);
        assert_eq!(unlock_amount, amount);
    }
}
