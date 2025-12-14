// Security tests for the backend

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::solana_client::{derive_vault_pda, derive_vault_authority_pda};
    use cvmsback::db;
    use solana_sdk::pubkey::Pubkey;
    use sqlx::Row;
    use std::str::FromStr;

    #[test]
    fn test_secure_pda_derivation() {
        let program_id = Pubkey::new_unique();
        let owner1 = Pubkey::new_unique();
        let owner2 = Pubkey::new_unique();
        
        // Derive PDAs
        let (pda1, bump1) = derive_vault_pda(&owner1, &program_id);
        let (pda2, bump2) = derive_vault_pda(&owner2, &program_id);
        let (pda1_again, bump1_again) = derive_vault_pda(&owner1, &program_id);
        
        // Same owner should produce same PDA
        assert_eq!(pda1, pda1_again);
        assert_eq!(bump1, bump1_again);
        
        // Different owners should produce different PDAs
        assert_ne!(pda1, pda2);
        
        // PDAs should be on-curve (valid pubkeys)
        // This is implicitly checked by Pubkey::find_program_address
    }

    #[test]
    fn test_vault_authority_pda_derivation() {
        let program_id = Pubkey::new_unique();
        
        let (authority1, bump1) = derive_vault_authority_pda(&program_id);
        let (authority2, bump2) = derive_vault_authority_pda(&program_id);
        
        // Should be deterministic
        assert_eq!(authority1, authority2);
        assert_eq!(bump1, bump2);
    }

    #[test]
    fn test_pda_deterministic_across_calls() {
        let program_id = Pubkey::from_str("5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT").unwrap();
        let owner = Pubkey::from_str("11111111111111111111111111111111").unwrap();
        
        // Derive multiple times
        let (pda1, bump1) = derive_vault_pda(&owner, &program_id);
        let (pda2, bump2) = derive_vault_pda(&owner, &program_id);
        let (pda3, bump3) = derive_vault_pda(&owner, &program_id);
        
        // All should be identical
        assert_eq!(pda1, pda2);
        assert_eq!(pda2, pda3);
        assert_eq!(bump1, bump2);
        assert_eq!(bump2, bump3);
    }

    #[tokio::test]
    #[ignore]
    async fn test_transaction_idempotency() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        let signature = "test_signature_123";
        
        // Insert same transaction twice
        insert_test_transaction(&ctx, &owner, signature, 1000, "deposit", "pending")
            .await.expect("Failed to insert");
        
        // Second insert should be idempotent (no error, but no duplicate)
        let result = db::insert_transaction_with_status(
            &ctx.pool, &owner, signature, Some(1000), "deposit", "pending"
        ).await;
        
        assert!(result.is_ok()); // Should not error
        
        // Verify only one transaction exists
        let transactions = db::list_transactions(&ctx.pool, &owner, 10, 0)
            .await.expect("Failed to list");
        
        assert_eq!(transactions.len(), 1);
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_audit_trail_completeness() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        
        // Perform various operations that should be audited
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        db::insert_audit_log(&ctx.pool, Some(&owner), "vault_created", serde_json::json!({}))
            .await.expect("Failed to insert audit");
        
        insert_test_transaction(&ctx, &owner, "sig1", 1000, "deposit", "pending")
            .await.expect("Failed to insert");
        
        db::insert_audit_log(&ctx.pool, Some(&owner), "deposit_initiated", serde_json::json!({"amount": 1000}))
            .await.expect("Failed to insert audit");
        
        // Verify audit trail
        let rows = sqlx::query(
            "SELECT COUNT(*) as count FROM audit_trail WHERE owner = $1"
        )
        .bind(&owner)
        .fetch_one(&ctx.pool)
        .await.expect("Failed to query");
        
        let count: i64 = rows.get::<i64, _>("count");
        assert!(count >= 2, "Audit trail should contain at least 2 entries");
        
        ctx.cleanup().await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_locked_balance_consistency() {
        let ctx = TestContext::new().await;
        let owner = TestContext::generate_test_owner();
        create_test_vault(&ctx, &owner).await.expect("Failed to create vault");
        
        // Set initial balance
        db::update_vault_snapshot(&ctx.pool, &owner, 100000, 0, 0)
            .await.expect("Failed to set balance");
        
        // Lock balance
        db::increment_locked_balance(&ctx.pool, &owner, 30000)
            .await.expect("Failed to lock");
        
        // Try to lock more than available (should be prevented by business logic)
        let _available = 100000i64 - 30000i64;
        // In real implementation, this check would be in the business logic
        // Here we just verify the locked balance doesn't exceed total
        
        let locked = db::get_locked_balance(&ctx.pool, &owner)
            .await.expect("Failed to get locked");
        let total = db::get_vault(&ctx.pool, &owner)
            .await.expect("Failed to get vault")
            .map(|(_, total)| total)
            .unwrap_or(0);
        
        assert!(locked <= total, "Locked balance should not exceed total");
        
        ctx.cleanup().await;
    }
}
