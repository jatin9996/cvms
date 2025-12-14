#[cfg(test)]
mod tests {
    use cvmsback::vault::VaultManager;
    use cvmsback::solana_client::{build_instruction_initialize_vault, build_instruction_deposit, DepositParams};
    use solana_sdk::pubkey::Pubkey;
    use std::str::FromStr;

    #[test]
    fn test_initialize_vault_instruction_builder() {
        let program_id = Pubkey::from_str("5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT").unwrap();
        let user = Pubkey::new_unique();
        let mint = Pubkey::from_str("4QHVBbG3H8kbwvcSwPnze3sC91kdeYWxNf8S5hkZ9nbZ").unwrap();
        
        let result = build_instruction_initialize_vault(&program_id, &user, &mint);
        assert!(result.is_ok());
        let ix = result.unwrap();
        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.accounts.len(), 8);
    }

    #[test]
    fn test_deposit_instruction_builder() {
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
        
        let result = build_instruction_deposit(&params);
        assert!(result.is_ok());
        let ix = result.unwrap();
        assert_eq!(ix.program_id, program_id);
        assert_eq!(ix.accounts.len(), 6);
        
        // Verify amount is encoded correctly
        let encoded_amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_deposit_instruction_with_different_amounts() {
        let program_id = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let mint = Pubkey::new_unique();
        
        for amount in [1u64, 100u64, 1000u64, 1000000u64, u64::MAX] {
            let params = DepositParams {
                program_id,
                owner,
                mint,
                amount,
            };
            
            let result = build_instruction_deposit(&params);
            assert!(result.is_ok());
            let ix = result.unwrap();
            let encoded_amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
            assert_eq!(encoded_amount, amount, "Amount encoding failed for {}", amount);
        }
    }
}
