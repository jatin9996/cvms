use crate::{api::AppState, db, error::{AppError, AppResult}, solana_client::{build_compute_budget_instructions, build_instruction_pm_lock, build_instruction_pm_unlock, load_deployer_keypair, send_transaction_with_retries}};
use solana_sdk::{pubkey::Pubkey, transaction::Transaction};

#[derive(Clone)]
pub struct CPIManager {
	state: AppState,
}

impl CPIManager {
	pub fn new(state: AppState) -> Self { Self { state } }

	pub async fn lock(&self, owner: &Pubkey, amount: u64) -> AppResult<String> {
		let pm_pid = Pubkey::from_str(&self.state.cfg.position_manager_program_id).unwrap_or_default();
		let vault_pid = Pubkey::from_str(&self.state.cfg.program_id).unwrap_or_default();
		let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
		let pm_ix = build_instruction_pm_lock(&pm_pid, &vault_pid, owner, amount)?;
		ixs.push(pm_ix);
		let payer = load_deployer_keypair(&self.state.cfg.deployer_keypair_path)?;
		let recent_blockhash = self.state.sol.rpc.get_latest_blockhash().await.map_err(|e| AppError::Solana(format!("blockhash: {e}")))?;
		let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
		let sig = send_transaction_with_retries(&self.state.sol, &tx, 3).await?;
		let _ = db::increment_locked_balance(&self.state.pool, &owner.to_string(), amount as i64).await;
		Ok(sig.to_string())
	}

	pub async fn unlock(&self, owner: &Pubkey, amount: u64) -> AppResult<String> {
		let pm_pid = Pubkey::from_str(&self.state.cfg.position_manager_program_id).unwrap_or_default();
		let vault_pid = Pubkey::from_str(&self.state.cfg.program_id).unwrap_or_default();
		let mut ixs = build_compute_budget_instructions(1_400_000, 1_000);
		let pm_ix = build_instruction_pm_unlock(&pm_pid, &vault_pid, owner, amount)?;
		ixs.push(pm_ix);
		let payer = load_deployer_keypair(&self.state.cfg.deployer_keypair_path)?;
		let recent_blockhash = self.state.sol.rpc.get_latest_blockhash().await.map_err(|e| AppError::Solana(format!("blockhash: {e}")))?;
		let tx = Transaction::new_signed_with_payer(&ixs, Some(&payer.pubkey()), &[&*payer], recent_blockhash);
		let sig = send_transaction_with_retries(&self.state.sol, &tx, 3).await?;
		let _ = db::increment_locked_balance(&self.state.pool, &owner.to_string(), -(amount as i64)).await;
		Ok(sig.to_string())
	}
}


