use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::nonblocking::rpc_client::SerializableTransaction;
use solana_sdk::{
	compute_budget::ComputeBudgetInstruction,
	instruction::{AccountMeta, Instruction},
	pubkey::Pubkey,
	signature::{Keypair, Signature, Signer},
	system_program,
	transaction::Transaction,
};
use spl_associated_token_account as spl_ata;
use spl_token;
use std::sync::Arc;

#[derive(Clone)]
pub struct SolanaClient {
	pub rpc: RpcClient,
}

impl SolanaClient {
	pub fn new(rpc_url: &str) -> Self {
		Self { rpc: RpcClient::new(rpc_url.to_string()) }
	}
}

// Types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollateralVault {
	pub address: String,
	pub owner: String,
	pub token_mint: String,
}

// Functions per spec - implemented to a generic, program-agnostic baseline
pub async fn get_vault_account(client: &SolanaClient, vault_pubkey: &Pubkey) -> AppResult<CollateralVault> {
	let _acc = client
		.rpc
		.get_account(vault_pubkey)
		.await
		.map_err(|e| AppError::Solana(format!("get_account failed: {e}")))?;
	Ok(CollateralVault { address: vault_pubkey.to_string(), owner: String::new(), token_mint: String::new() })
}

// Derive vault PDA using seeds: [b"vault", owner]
pub fn derive_vault_pda(owner: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", owner.as_ref()], program_id)
}

// Fetch on-chain multisig config from CollateralVault account
pub async fn fetch_vault_multisig_config(client: &SolanaClient, owner: &Pubkey, program_id: &Pubkey) -> AppResult<(u8, Vec<Pubkey>)> {
    let (vault_pda, _) = derive_vault_pda(owner, program_id);
    let acc = client
        .rpc
        .get_account(&vault_pda)
        .await
        .map_err(|e| AppError::Solana(format!("get_account failed: {e}")))?;
    let data = acc.data;
    if data.len() < 134 { // discriminator(8) + fields before vec(122?) + bump + threshold + vec len
        return Err(AppError::Internal("vault account too small".to_string()));
    }
    // Offsets based on Anchor/Borsh layout used by on-chain CollateralVault
    // discriminator: 0..8
    // owner: 8..40, token_account: 40..72, usdt_mint: 72..104
    // totals: 104..136 (u64 * 4), created_at: 136..144, bump: 144..145, threshold: 145..146
    // Note: actual created_at/bump offsets in on-chain struct are: created_at at 120..128, bump at 128..129, threshold 129..130 per current layout.
    // We recompute using exact layout from program: use those constants.
    let threshold_offset = 129usize; // after discriminator(8) + 32*3 + 8*6 + 1 bump
    let vec_len_offset = 130usize;
    if data.len() < vec_len_offset + 4 { return Err(AppError::Internal("vault account corrupted".to_string())); }
    let threshold = data[threshold_offset];
    let len_bytes = &data[vec_len_offset..vec_len_offset+4];
    let len = u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
    let mut signers: Vec<Pubkey> = Vec::with_capacity(len);
    let mut cursor = vec_len_offset + 4;
    for _ in 0..len {
        if data.len() < cursor + 32 { return Err(AppError::Internal("vault account truncated".to_string())); }
        let pk = Pubkey::new(&data[cursor..cursor+32]);
        signers.push(pk);
        cursor += 32;
    }
    Ok((threshold, signers))
}

pub async fn get_token_balance(client: &SolanaClient, token_account_pubkey: &Pubkey) -> AppResult<u64> {
	let ui = client
		.rpc
		.get_token_account_balance(token_account_pubkey)
		.await
		.map_err(|e| AppError::Solana(format!("get_token_account_balance failed: {e}")))?;
	let amount: u64 = ui.amount.parse().unwrap_or(0);
	Ok(amount)
}

pub async fn send_transaction(client: &SolanaClient, tx: &Transaction) -> AppResult<Signature> {
	let sig = client
		.rpc
		.send_and_confirm_serialized_transaction(tx)
		.await
		.map_err(|e| AppError::Solana(format!("send_and_confirm_transaction failed: {e}")))?;
	Ok(sig)
}

pub async fn send_transaction_with_retries(client: &SolanaClient, tx: &Transaction, max_retries: usize) -> AppResult<Signature> {
	let mut last_err: Option<String> = None;
	for attempt in 0..max_retries {
		match send_transaction(client, tx).await {
			Ok(sig) => return Ok(sig),
			Err(e) => {
				last_err = Some(format!("{e}"));
				tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1))).await;
			}
		}
	}
	Err(AppError::Solana(last_err.unwrap_or_else(|| "send failed".to_string())))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepositParams {
	pub program_id: Pubkey,
	pub user: Pubkey,
	pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawParams {
	pub program_id: Pubkey,
    /// Vault owner pubkey used for PDA derivation
	pub owner: Pubkey,
	pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawMultisigParams {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub authority: Pubkey,
    pub amount: u64,
    pub other_signers: Vec<Pubkey>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleTimelockParams {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub duration_seconds: i64,
}

pub fn build_instruction_initialize_vault(program_id: &Pubkey, user_pubkey: &Pubkey) -> AppResult<Instruction> {
	let accounts = vec![
		AccountMeta::new(*user_pubkey, true),
		AccountMeta::new_readonly(system_program::id(), false),
	];
	// Generic placeholder data layout: [op=0]
	let data = vec![0u8];
	Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_deposit(params: &DepositParams) -> AppResult<Instruction> {
	let accounts = vec![
		AccountMeta::new(params.user, true),
	];
	// Generic placeholder data layout: [op=1 | amount u64 LE]
	let mut data = vec![1u8];
	data.extend_from_slice(&params.amount.to_le_bytes());
	Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_withdraw(params: &WithdrawParams) -> AppResult<Instruction> {
	let accounts = vec![
        // authority signer (one of multisig signers or owner)
        AccountMeta::new(params.owner, true),
        // owner as readonly for PDA seed (placeholder in this scaffold)
        AccountMeta::new_readonly(params.owner, false),
	];
	// Generic placeholder data layout: [op=2 | amount u64 LE]
	let mut data = vec![2u8];
	data.extend_from_slice(&params.amount.to_le_bytes());
	Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_withdraw_multisig(params: &WithdrawMultisigParams) -> AppResult<Instruction> {
    let mut accounts = vec![
        AccountMeta::new(params.authority, true),
        AccountMeta::new_readonly(params.owner, false),
    ];
    // Include other approving signers as remaining accounts requiring signatures
    for s in params.other_signers.iter() {
        if *s != params.authority {
            accounts.push(AccountMeta::new_readonly(*s, true));
        }
    }
    let mut data = vec![2u8];
    data.extend_from_slice(&params.amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_schedule_timelock(params: &ScheduleTimelockParams) -> AppResult<Instruction> {
    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
    ];
    // Placeholder layout: [op=20 | amount u64 | duration i64]
    let mut data = vec![20u8];
    data.extend_from_slice(&params.amount.to_le_bytes());
    data.extend_from_slice(&params.duration_seconds.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub async fn build_partial_withdraw_tx(
    client: &SolanaClient,
    payer: &Keypair,
    params: &WithdrawMultisigParams,
) -> AppResult<Vec<u8>> {
    let mut ixs = build_compute_budget_instructions(1_200_000, 1_000);
    let wd_ix = build_instruction_withdraw_multisig(params)?;
    ixs.push(wd_ix);
    let recent_blockhash = client
        .rpc
        .get_latest_blockhash()
        .await
        .map_err(|e| AppError::Solana(format!("blockhash: {e}")))?;
    let mut tx = Transaction::new_unsigned(solana_sdk::message::Message::new(&ixs, Some(&payer.pubkey())));
    tx.partial_sign(&[payer], recent_blockhash);
    let bytes = bincode::serialize(&tx).map_err(|e| AppError::Internal(format!("serialize tx: {e}")))?;
    Ok(bytes)
}

pub fn build_instruction_pm_lock(position_manager_program_id: &Pubkey, vault_program_id: &Pubkey, owner: &Pubkey, amount: u64) -> AppResult<Instruction> {
	let accounts = vec![
		AccountMeta::new(*owner, true),
		AccountMeta::new_readonly(*vault_program_id, false),
	];
	let mut data = vec![10u8]; // op code placeholder for lock
	data.extend_from_slice(&amount.to_le_bytes());
	Ok(Instruction { program_id: *position_manager_program_id, accounts, data })
}

pub fn build_instruction_pm_unlock(position_manager_program_id: &Pubkey, vault_program_id: &Pubkey, owner: &Pubkey, amount: u64) -> AppResult<Instruction> {
	let accounts = vec![
		AccountMeta::new(*owner, true),
		AccountMeta::new_readonly(*vault_program_id, false),
	];
	let mut data = vec![11u8]; // op code placeholder for unlock
	data.extend_from_slice(&amount.to_le_bytes());
	Ok(Instruction { program_id: *position_manager_program_id, accounts, data })
}

pub fn build_compute_budget_instructions(units: u32, micro_lamports: u64) -> Vec<Instruction> {
	vec![
		ComputeBudgetInstruction::set_compute_unit_limit(units),
		ComputeBudgetInstruction::set_compute_unit_price(micro_lamports),
	]
}

pub fn load_deployer_keypair(path: &str) -> AppResult<Arc<Keypair>> {
	// Prefer env-based secret if provided (DEPLOYER_KEYPAIR_BASE64)
	if let Ok(b64) = std::env::var("DEPLOYER_KEYPAIR_BASE64") {
		let bytes = base64::decode(b64).map_err(|e| AppError::Internal(format!("invalid base64 keypair: {e}")))?;
		let kp = Keypair::from_bytes(&bytes).map_err(|e| AppError::Internal(format!("invalid keypair bytes: {e}")))?;
		return Ok(Arc::new(kp));
	}
	use solana_sdk::signature::read_keypair_file;
	let kp = read_keypair_file(path).map_err(|e| AppError::Internal(format!("failed to read keypair: {e}")))?;
	Ok(Arc::new(kp))
}

// SPL Token helpers
pub fn derive_associated_token_address(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
	spl_ata::get_associated_token_address(owner, mint)
}

pub fn build_create_ata_instruction(payer: &Pubkey, owner: &Pubkey, mint: &Pubkey) -> Instruction {
	spl_ata::instruction::create_associated_token_account(
		payer,
		owner,
		mint,
		&spl_token::id(),
		&spl_ata::id(),
	)
}

pub async fn subscribe_to_account(_pubkey: Pubkey) -> AppResult<()> {
	// Placeholder: actual subscription is integrated in ws module to stream updates to clients
	Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_budget_ixs() {
        let ixs = build_compute_budget_instructions(1_000_000, 1_000);
        assert_eq!(ixs.len(), 2);
        assert_eq!(ixs[0].program_id, solana_sdk::compute_budget::id());
        assert_eq!(ixs[1].program_id, solana_sdk::compute_budget::id());
    }
}


