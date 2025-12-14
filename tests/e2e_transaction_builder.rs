// End-to-end tests for Transaction Builder

mod test_utils;

#[cfg(test)]
mod tests {
    use crate::test_utils::*;
    use cvmsback::solana_client::{
        build_compute_budget_instructions, build_instruction_deposit, build_instruction_withdraw,
        build_instruction_initialize_vault, DepositParams, WithdrawParams,
    };
    use solana_sdk::{
        compute_budget::ComputeBudgetInstruction,
        instruction::Instruction,
        pubkey::Pubkey,
    };
    use std::str::FromStr;

    #[test]
    fn test_deposit_transaction_building() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 5000u64;

        let params = DepositParams {
            program_id,
            owner,
            mint,
            amount,
        };

        let deposit_ix = build_instruction_deposit(&params).expect("Failed to build deposit instruction");

        // Verify instruction structure
        assert_eq!(deposit_ix.program_id, program_id);
        assert_eq!(deposit_ix.accounts.len(), 6);
        
        // Verify amount encoding
        let encoded_amount = u64::from_le_bytes(deposit_ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_withdraw_transaction_building() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 3000u64;

        let params = WithdrawParams {
            program_id,
            owner,
            mint,
            amount,
        };

        let withdraw_ix = build_instruction_withdraw(&params).expect("Failed to build withdraw instruction");

        assert_eq!(withdraw_ix.program_id, program_id);
        assert_eq!(withdraw_ix.accounts.len(), 6);
        
        let encoded_amount = u64::from_le_bytes(withdraw_ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_compute_budget_instructions() {
        let units = 1_400_000u32;
        let micro_lamports = 1_000u64;

        let ixs = build_compute_budget_instructions(units, micro_lamports);

        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, solana_sdk::compute_budget::id());
        assert_eq!(ixs[1].program_id, solana_sdk::compute_budget::id());
    }

    #[test]
    fn test_complete_transaction_with_compute_budget() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 10000u64;

        // Build complete transaction
        let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
        
        let deposit_params = DepositParams {
            program_id,
            owner,
            mint,
            amount,
        };
        let deposit_ix = build_instruction_deposit(&deposit_params)
            .expect("Failed to build deposit instruction");
        ixs.push(deposit_ix);

        assert_eq!(ixs.len(), 3); // 2 compute budget + 1 deposit
        assert_eq!(ixs[2].program_id, program_id);
    }

    #[test]
    fn test_initialize_vault_instruction() {
        let program_id = Pubkey::from_str("5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT").unwrap();
        let user = Pubkey::new_unique();
        let mint = Pubkey::from_str("4QHVBbG3H8kbwvcSwPnze3sC91kdeYWxNf8S5hkZ9nbZ").unwrap();

        let init_ix = build_instruction_initialize_vault(&program_id, &user, &mint)
            .expect("Failed to build initialize instruction");

        assert_eq!(init_ix.program_id, program_id);
        assert_eq!(init_ix.accounts.len(), 8);
        
        // Verify required accounts are present
        assert!(init_ix.accounts.iter().any(|a| a.pubkey == user && a.is_signer));
    }

    #[test]
    fn test_spl_token_account_handling() {
        use cvmsback::solana_client::derive_associated_token_address;
        
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        
        let ata = derive_associated_token_address(&owner, &mint);
        
        // ATA should be deterministic
        let ata2 = derive_associated_token_address(&owner, &mint);
        assert_eq!(ata, ata2);
        
        // Different owner should produce different ATA
        let owner2 = Pubkey::new_unique();
        let ata3 = derive_associated_token_address(&owner2, &mint);
        assert_ne!(ata, ata3);
    }

    #[test]
    fn test_transaction_builder_with_various_amounts() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();

        let test_amounts = vec![1u64, 100u64, 1000u64, 1000000u64, u64::MAX / 2];

        for amount in test_amounts {
            let params = DepositParams {
                program_id,
                owner,
                mint,
                amount,
            };

            let ix = build_instruction_deposit(&params)
                .expect(&format!("Failed to build instruction for amount {}", amount));
            
            let encoded = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
            assert_eq!(encoded, amount, "Amount encoding failed for {}", amount);
        }
    }
}
