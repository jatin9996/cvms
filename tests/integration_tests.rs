#[cfg(test)]
mod tests {
    use cvmsback::solana_client::{
        build_instruction_deposit, build_instruction_initialize_vault, build_instruction_withdraw,
        DepositParams, WithdrawParams,
    };
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
}
