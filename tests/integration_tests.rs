#[cfg(test)]
mod tests {
    use cvmsback::solana_client::{
        build_instruction_deposit, build_instruction_initialize_vault,
        build_instruction_transfer_collateral, build_instruction_withdraw,
        derive_associated_token_address, derive_vault_authority_pda, derive_vault_pda,
        DepositParams, TransferCollateralParams, WithdrawParams,
    };
    use solana_sdk::instruction::AccountMeta;
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    // Integration test for vault initialization flow
    #[test]
    fn test_vault_initialization_flow() {
        let program_id = Pubkey::from_str("5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT").unwrap();
        let user = Pubkey::new_unique();
        let mint = Pubkey::from_str("4QHVBbG3H8kbwvcSwPnze3sC91kdeYWxNf8S5hkZ9nbZ").unwrap();

        // Step 1: Initialize vault
        let init_ix = build_instruction_initialize_vault(&program_id, &user, &mint).unwrap();
        assert_eq!(init_ix.program_id, program_id);
        assert_eq!(init_ix.accounts.len(), 8);
    }

    // Integration test for deposit flow
    #[test]
    fn test_deposit_flow() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 1000u64;

        let params = DepositParams {
            program_id,
            owner,
            mint,
            amount,
        };

        let deposit_ix = build_instruction_deposit(&params).unwrap();
        assert_eq!(deposit_ix.program_id, program_id);
        assert_eq!(deposit_ix.accounts.len(), 6);

        // Verify amount encoding
        let encoded_amount = u64::from_le_bytes(deposit_ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    // Integration test for withdrawal flow
    #[test]
    fn test_withdrawal_flow() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 500u64;

        let params = WithdrawParams {
            program_id,
            owner,
            mint,
            amount,
        };

        let withdraw_ix = build_instruction_withdraw(&params).unwrap();
        assert_eq!(withdraw_ix.program_id, program_id);
        assert_eq!(withdraw_ix.accounts.len(), 6);

        // Verify amount encoding
        let encoded_amount = u64::from_le_bytes(withdraw_ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    // Integration test for complete vault lifecycle
    #[test]
    fn test_vault_lifecycle() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();

        // 1. Initialize
        let init_ix = build_instruction_initialize_vault(&program_id, &owner, &mint).unwrap();
        assert_eq!(init_ix.program_id, program_id);

        // 2. Deposit
        let deposit_params = DepositParams {
            program_id,
            owner,
            mint,
            amount: 1000,
        };
        let deposit_ix = build_instruction_deposit(&deposit_params).unwrap();
        assert_eq!(deposit_ix.program_id, program_id);

        // 3. Withdraw
        let withdraw_params = WithdrawParams {
            program_id,
            owner,
            mint,
            amount: 500,
        };
        let withdraw_ix = build_instruction_withdraw(&withdraw_params).unwrap();
        assert_eq!(withdraw_ix.program_id, program_id);

        // All instructions should reference the same program and owner
        assert_eq!(init_ix.program_id, deposit_ix.program_id);
        assert_eq!(deposit_ix.program_id, withdraw_ix.program_id);
    }

    // --- SPL Token / transfer_collateral integration tests ---

    #[test]
    fn test_transfer_collateral_flow() {
        let program_id = Pubkey::new_unique();
        let caller_program = Pubkey::new_unique();
        let from_owner = Pubkey::new_unique();
        let to_owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let amount = 1_000_000u64;

        let params = TransferCollateralParams {
            program_id,
            caller_program,
            from_owner,
            to_owner,
            mint,
            amount,
        };

        let ix = build_instruction_transfer_collateral(&params).unwrap();
        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.accounts.len(), 8);

        let (from_vault, _) = derive_vault_pda(&from_owner, &program_id);
        let (to_vault, _) = derive_vault_pda(&to_owner, &program_id);
        let from_vault_ata = derive_associated_token_address(&from_vault, &mint);
        let to_vault_ata = derive_associated_token_address(&to_vault, &mint);
        let (va, _) = derive_vault_authority_pda(&program_id);

        assert_eq!(ix.accounts[0], AccountMeta::new_readonly(caller_program, false));
        assert_eq!(ix.accounts[1], AccountMeta::new_readonly(va, false));
        assert_eq!(ix.accounts[2], AccountMeta::new_readonly(solana_sdk::sysvar::instructions::ID, false));
        assert_eq!(ix.accounts[3], AccountMeta::new(from_vault, false));
        assert_eq!(ix.accounts[4], AccountMeta::new(to_vault, false));
        assert_eq!(ix.accounts[5], AccountMeta::new(from_vault_ata, false));
        assert_eq!(ix.accounts[6], AccountMeta::new(to_vault_ata, false));
        // 8th account is SPL Token program (readonly)
        assert!(!ix.accounts[7].is_writable);

        let encoded_amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_spl_token_transfer_collateral_amount_encoding() {
        let program_id = Pubkey::new_unique();
        let params = TransferCollateralParams {
            program_id,
            caller_program: Pubkey::new_unique(),
            from_owner: Pubkey::new_unique(),
            to_owner: Pubkey::new_unique(),
            mint: Pubkey::new_unique(),
            amount: 42_u64,
        };
        let ix = build_instruction_transfer_collateral(&params).unwrap();
        let amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
        assert_eq!(amount, 42);
    }

    #[test]
    fn test_spl_token_ata_derivation_consistency() {
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let ata = derive_associated_token_address(&owner, &mint);
        let ata2 = derive_associated_token_address(&owner, &mint);
        assert_eq!(ata, ata2);
        let other_mint = Pubkey::new_unique();
        let ata_other = derive_associated_token_address(&owner, &other_mint);
        assert_ne!(ata, ata_other);
    }

    #[test]
    fn test_spl_token_lifecycle_with_transfer_collateral() {
        let program_id = Pubkey::new_unique();
        let owner_a = Pubkey::new_unique();
        let owner_b = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        let caller = Pubkey::new_unique();

        let init_ix = build_instruction_initialize_vault(&program_id, &owner_a, &mint).unwrap();
        let deposit_ix = build_instruction_deposit(&DepositParams {
            program_id,
            owner: owner_a,
            mint,
            amount: 1000,
        })
        .unwrap();
        let transfer_ix = build_instruction_transfer_collateral(&TransferCollateralParams {
            program_id,
            caller_program: caller,
            from_owner: owner_a,
            to_owner: owner_b,
            mint,
            amount: 400,
        })
        .unwrap();
        let withdraw_ix = build_instruction_withdraw(&WithdrawParams {
            program_id,
            owner: owner_b,
            mint,
            amount: 400,
        })
        .unwrap();

        assert_eq!(init_ix.program_id, program_id);
        assert_eq!(deposit_ix.program_id, program_id);
        assert_eq!(transfer_ix.program_id, program_id);
        assert_eq!(withdraw_ix.program_id, program_id);
        assert_eq!(transfer_ix.accounts.len(), 8);
    }
}
