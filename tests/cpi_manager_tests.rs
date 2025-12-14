#[cfg(test)]
mod tests {
    use cvmsback::solana_client::{build_instruction_pm_lock, build_instruction_pm_unlock};
    use solana_sdk::pubkey::Pubkey;

    #[test]
    fn test_pm_lock_instruction_builder() {
        let pm_program = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 5000u64;
        
        let result = build_instruction_pm_lock(&pm_program, &vault_program, &owner, amount);
        assert!(result.is_ok());
        let ix = result.unwrap();
        assert_eq!(ix.program_id, pm_program);
        assert_eq!(ix.accounts.len(), 2);
        assert_eq!(ix.data[0], 10); // op code for lock
        let encoded_amount = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_pm_unlock_instruction_builder() {
        let pm_program = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 3000u64;
        
        let result = build_instruction_pm_unlock(&pm_program, &vault_program, &owner, amount);
        assert!(result.is_ok());
        let ix = result.unwrap();
        assert_eq!(ix.program_id, pm_program);
        assert_eq!(ix.accounts.len(), 2);
        assert_eq!(ix.data[0], 11); // op code for unlock
        let encoded_amount = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
        assert_eq!(encoded_amount, amount);
    }

    #[test]
    fn test_lock_unlock_consistency() {
        let pm_program = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 10000u64;
        
        let lock_ix = build_instruction_pm_lock(&pm_program, &vault_program, &owner, amount).unwrap();
        let unlock_ix = build_instruction_pm_unlock(&pm_program, &vault_program, &owner, amount).unwrap();
        
        // Both should have same program and owner
        assert_eq!(lock_ix.program_id, unlock_ix.program_id);
        assert_eq!(lock_ix.accounts[0].pubkey, unlock_ix.accounts[0].pubkey);
        
        // But different op codes
        assert_ne!(lock_ix.data[0], unlock_ix.data[0]);
    }
}
