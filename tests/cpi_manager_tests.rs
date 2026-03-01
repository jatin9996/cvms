#[cfg(test)]
mod tests {
    use cvmsback::solana_client::{
        build_instruction_mock_pm_close_position, build_instruction_mock_pm_open_position,
        build_instruction_pm_lock, build_instruction_pm_unlock,
        derive_position_summary_pda, derive_vault_authority_pda, derive_vault_pda,
    };
    use solana_sdk::sysvar;
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

        let lock_ix =
            build_instruction_pm_lock(&pm_program, &vault_program, &owner, amount).unwrap();
        let unlock_ix =
            build_instruction_pm_unlock(&pm_program, &vault_program, &owner, amount).unwrap();

        // Both should have same program and owner
        assert_eq!(lock_ix.program_id, unlock_ix.program_id);
        assert_eq!(lock_ix.accounts[0].pubkey, unlock_ix.accounts[0].pubkey);

        // But different op codes
        assert_ne!(lock_ix.data[0], unlock_ix.data[0]);
    }

    // --- Mock position manager (CPI-compatible) tests ---

    #[test]
    fn test_mock_pm_open_position_instruction_layout() {
        let mock_pm = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 100_000u64;

        let ix = build_instruction_mock_pm_open_position(&mock_pm, &vault_program, &owner, amount)
            .expect("build mock pm open_position");
        let (vault_pda, _) = derive_vault_pda(&owner, &vault_program);
        let (va_pda, _) = derive_vault_authority_pda(&vault_program);

        assert_eq!(ix.program_id, mock_pm);
        assert_eq!(ix.accounts.len(), 6, "mock open_position has 6 accounts");
        assert_eq!(ix.accounts[0].pubkey, mock_pm);
        assert_eq!(ix.accounts[1].pubkey, va_pda);
        assert_eq!(ix.accounts[2].pubkey, sysvar::instructions::ID);
        assert_eq!(ix.accounts[3].pubkey, vault_pda);
        assert_eq!(ix.accounts[4].pubkey, derive_position_summary_pda(&vault_pda, &mock_pm));
        assert_eq!(ix.accounts[5].pubkey, vault_program);
        assert_eq!(ix.data.len(), 8 + 8, "discriminator + u64 amount");
        assert_eq!(
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            amount
        );
    }

    #[test]
    fn test_mock_pm_close_position_instruction_layout() {
        let mock_pm = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 50_000u64;

        let ix = build_instruction_mock_pm_close_position(&mock_pm, &vault_program, &owner, amount)
            .expect("build mock pm close_position");
        let (vault_pda, _) = derive_vault_pda(&owner, &vault_program);
        let (va_pda, _) = derive_vault_authority_pda(&vault_program);

        assert_eq!(ix.program_id, mock_pm);
        assert_eq!(ix.accounts.len(), 6);
        assert_eq!(ix.accounts[0].pubkey, mock_pm);
        assert_eq!(ix.accounts[1].pubkey, va_pda);
        assert_eq!(ix.accounts[2].pubkey, sysvar::instructions::ID);
        assert_eq!(ix.accounts[3].pubkey, vault_pda);
        assert_eq!(ix.accounts[4].pubkey, derive_position_summary_pda(&vault_pda, &mock_pm));
        assert_eq!(ix.accounts[5].pubkey, vault_program);
        assert_eq!(ix.data.len(), 8 + 8);
        assert_eq!(
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            amount
        );
    }

    #[test]
    fn test_mock_pm_open_close_discriminators_differ() {
        let mock_pm = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 1u64;

        let open_ix =
            build_instruction_mock_pm_open_position(&mock_pm, &vault_program, &owner, amount)
                .unwrap();
        let close_ix =
            build_instruction_mock_pm_close_position(&mock_pm, &vault_program, &owner, amount)
                .unwrap();
        assert_ne!(
            open_ix.data[..8],
            close_ix.data[..8],
            "open_position and close_position must have different Anchor discriminators"
        );
    }

    #[test]
    fn test_mock_pm_position_summary_pda_consistent() {
        let mock_pm = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let (vault_pda, _) = derive_vault_pda(&owner, &vault_program);

        let open_ix =
            build_instruction_mock_pm_open_position(&mock_pm, &vault_program, &owner, 1).unwrap();
        let close_ix =
            build_instruction_mock_pm_close_position(&mock_pm, &vault_program, &owner, 1).unwrap();
        let expected_summary = derive_position_summary_pda(&vault_pda, &mock_pm);
        assert_eq!(open_ix.accounts[4].pubkey, expected_summary);
        assert_eq!(close_ix.accounts[4].pubkey, expected_summary);
    }

    #[test]
    fn test_mock_pm_instruction_consistency_with_simplified_opcodes() {
        // Simplified PM uses opcode 10/11; mock PM uses Anchor discriminators.
        // Both encode amount as little-endian u64 after the first 8 bytes (mock) or 1 byte (simple).
        let mock_pm = Pubkey::new_unique();
        let vault_program = Pubkey::new_unique();
        let owner = Pubkey::new_unique();
        let amount = 99u64;
        let mock_ix =
            build_instruction_mock_pm_open_position(&mock_pm, &vault_program, &owner, amount)
                .unwrap();
        let simple_ix =
            build_instruction_pm_lock(&mock_pm, &vault_program, &owner, amount).unwrap();
        assert_eq!(
            u64::from_le_bytes(mock_ix.data[8..16].try_into().unwrap()),
            amount
        );
        assert_eq!(
            u64::from_le_bytes(simple_ix.data[1..9].try_into().unwrap()),
            amount
        );
        assert_ne!(mock_ix.accounts.len(), simple_ix.accounts.len());
    }
}
