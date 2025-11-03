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
	pub owner: Pubkey,
	pub amount: u64,
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
		AccountMeta::new(params.owner, true),
	];
	// Generic placeholder data layout: [op=2 | amount u64 LE]
	let mut data = vec![2u8];
	data.extend_from_slice(&params.amount.to_le_bytes());
	Ok(Instruction { program_id: params.program_id, accounts, data })
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


