use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, signature::Signature, transaction::Transaction};

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

// Functions per spec - stubs to be implemented
pub async fn get_vault_account(_client: &SolanaClient, _vault_pubkey: &Pubkey) -> AppResult<CollateralVault> {
	Ok(CollateralVault { address: _vault_pubkey.to_string(), owner: String::new(), token_mint: String::new() })
}

pub async fn get_token_balance(_client: &SolanaClient, _token_account_pubkey: &Pubkey) -> AppResult<u64> {
	Ok(0)
}

pub async fn send_transaction(_client: &SolanaClient, _tx: &Transaction) -> AppResult<Signature> {
	Err(AppError::Solana("send_transaction not implemented".to_string()))
}

pub fn build_instruction_initialize_vault(_user_pubkey: &Pubkey) -> AppResult<Instruction> {
	Err(AppError::BadRequest("initialize_vault not implemented".to_string()))
}

pub fn build_instruction_deposit(_params: ()) -> AppResult<Instruction> {
	Err(AppError::BadRequest("deposit not implemented".to_string()))
}

pub fn build_instruction_withdraw(_params: ()) -> AppResult<Instruction> {
	Err(AppError::BadRequest("withdraw not implemented".to_string()))
}

pub async fn subscribe_to_account(_pubkey: Pubkey) -> AppResult<()> {
	Ok(())
}


