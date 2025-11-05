use crate::{api::AppState, db, error::{AppError, AppResult}, solana_client::{build_compute_budget_instructions, build_instruction_deposit, build_instruction_initialize_vault, build_instruction_withdraw, DepositParams, WithdrawParams, send_transaction_with_retries, load_deployer_keypair}};
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, transaction::Transaction};

#[derive(Clone)]
pub struct VaultManager {
	state: AppState,
}

impl VaultManager {
	pub fn new(state: AppState) -> Self { Self { state } }

	pub fn build_initialize_vault_ix(&self, user_pubkey: &Pubkey) -> AppResult<Instruction> {
		let program_id = Pubkey::from_str(&self.state.cfg.program_id).unwrap_or_default();
		let mint = Pubkey::from_str(&self.state.cfg.usdt_mint).unwrap_or_default();
		build_instruction_initialize_vault(&program_id, user_pubkey, &mint)
	}

	pub fn build_deposit_ix(&self, owner: &Pubkey, amount: u64) -> AppResult<Instruction> {
		let program_id = Pubkey::from_str(&self.state.cfg.program_id).unwrap_or_default();
		let mint = Pubkey::from_str(&self.state.cfg.usdt_mint).unwrap_or_default();
		let params = DepositParams { program_id, owner: *owner, mint, amount };
		build_instruction_deposit(&params)
	}

	pub async fn submit_withdraw(&self, owner: &Pubkey, amount: u64) -> AppResult<String> {
		let program_id = Pubkey::from_str(&self.state.cfg.program_id).unwrap_or_default();
		let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
		let mint = Pubkey::from_str(&self.state.cfg.usdt_mint).unwrap_or_default();
		let wd_ix = build_instruction_withdraw(&WithdrawParams { program_id, owner: *owner, mint, amount })?;
		ixs.push(wd_ix);
		let payer = load_deployer_keypair(&self.state.cfg.deployer_keypair_path)?;
		let recent_blockhash = self.state.sol.rpc.get_latest_blockhash().await.map_err(|e| AppError::Solana(format!("blockhash: {e}")))?;
		let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
		let sig = send_transaction_with_retries(&self.state.sol, &tx, 3).await?;
		Ok(sig.to_string())
	}

	pub async fn query_balance_by_owner(&self, owner: &str) -> AppResult<u64> {
		// Prefer DB-mapped token account; fallback to interpreting owner as token account
		if let Some((token_acc_opt, _)) = db::get_vault(&self.state.pool, owner).await? {
			if let Some(token_acc) = token_acc_opt {
				if let Ok(pk) = Pubkey::from_str(&token_acc) {
					return crate::solana_client::get_token_balance(&self.state.sol, &pk).await;
				}
			}
		}
		if let Ok(pk) = Pubkey::from_str(owner) {
			return crate::solana_client::get_token_balance(&self.state.sol, &pk).await;
		}
		Err(AppError::BadRequest("invalid owner or token account".to_string()))
	}

	pub async fn available_balance(&self, owner: &str) -> AppResult<i64> {
		let (_token_opt, total) = db::get_vault(&self.state.pool, owner).await?.unwrap_or((None, 0));
		let locked = db::get_locked_balance(&self.state.pool, owner).await.unwrap_or(0);
		Ok(total - locked)
	}
}


