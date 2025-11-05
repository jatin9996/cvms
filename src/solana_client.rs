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
use sha2::{Digest, Sha256};
use solana_program::sysvar;
use borsh::BorshDeserialize;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultOnchainSnapshot {
    pub owner: Pubkey,
    pub total_balance: u64,
    pub locked_balance: u64,
    pub available_balance: u64,
    pub yield_deposited_balance: u64,
    pub yield_accrued_balance: u64,
    pub active_yield_program: Pubkey,
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

pub async fn fetch_vault_yield_info(client: &SolanaClient, owner: &Pubkey, program_id: &Pubkey) -> AppResult<VaultOnchainSnapshot> {
    let (vault_pda, _) = derive_vault_pda(owner, program_id);
    let acc = client.rpc.get_account(&vault_pda).await.map_err(|e| AppError::Solana(format!("get_account failed: {e}")))?;
    let data = acc.data;
    if data.len() < 8 + 32*3 + 8*11 + 32 + 8 + 1 + 1 { // coarse length check
        return Err(AppError::Internal("vault account too small".to_string()));
    }
    let mut cursor = 8usize; // skip discriminator
    let owner_pk = Pubkey::new(&data[cursor..cursor+32]); cursor += 32;
    // token_account
    cursor += 32;
    // usdt_mint
    cursor += 32;
    let mut read_u64 = |cur: &mut usize| -> u64 {
        let v = u64::from_le_bytes(data[*cur..*cur+8].try_into().unwrap());
        *cur += 8; v
    };
    let total_balance = read_u64(&mut cursor);
    let locked_balance = read_u64(&mut cursor);
    let available_balance = read_u64(&mut cursor);
    // totals
    cursor += 8; // total_deposited
    cursor += 8; // total_withdrawn
    let yield_deposited_balance = read_u64(&mut cursor);
    let yield_accrued_balance = read_u64(&mut cursor);
    // last_compounded_at (i64)
    cursor += 8;
    let active_yield_program = Pubkey::new(&data[cursor..cursor+32]); cursor += 32;
    // created_at (i64)
    cursor += 8;
    // bump
    cursor += 1;
    // multisig_threshold
    cursor += 1;
    Ok(VaultOnchainSnapshot {
        owner: owner_pk,
        total_balance,
        locked_balance,
        available_balance,
        yield_deposited_balance,
        yield_accrued_balance,
        active_yield_program,
    })
}

// Derive vault PDA using seeds: [b"vault", owner]
pub fn derive_vault_pda(owner: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", owner.as_ref()], program_id)
}

pub fn derive_vault_authority_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault_authority"], program_id)
}

fn anchor_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(format!("global:{}", name));
    let hash = hasher.finalize();
    let mut disc = [0u8; 8];
    disc.copy_from_slice(&hash[..8]);
    disc
}

// Fetch on-chain multisig config from CollateralVault account
pub async fn fetch_vault_multisig_config(client: &SolanaClient, owner: &Pubkey, program_id: &Pubkey) -> AppResult<(u8, Vec<Pubkey>)> {
    #[derive(BorshDeserialize)]
    struct Head {
        owner: [u8; 32],
        token_account: [u8; 32],
        usdt_mint: [u8; 32],
        total_balance: u64,
        locked_balance: u64,
        available_balance: u64,
        total_deposited: u64,
        total_withdrawn: u64,
        yield_deposited_balance: u64,
        yield_accrued_balance: u64,
        last_compounded_at: i64,
        active_yield_program: [u8; 32],
        created_at: i64,
        bump: u8,
        multisig_threshold: u8,
        multisig_signers: Vec<[u8; 32]>,
    }

    let (vault_pda, _) = derive_vault_pda(owner, program_id);
    let acc = client
        .rpc
        .get_account(&vault_pda)
        .await
        .map_err(|e| AppError::Solana(format!("get_account failed: {e}")))?;
    let data = acc.data;
    if data.len() < 8 { return Err(AppError::Internal("vault account too small".to_string())); }
    let mut slice: &[u8] = &data[8..];
    let head = Head::deserialize(&mut slice).map_err(|_| AppError::Internal("decode vault head failed".to_string()))?;
    let threshold = head.multisig_threshold;
    let signers: Vec<Pubkey> = head.multisig_signers.into_iter().map(Pubkey::new_from_array).collect();
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
    /// Vault owner; also used as authority in single-owner mode
    pub owner: Pubkey,
    /// SPL mint for collateral (USDT)
    pub mint: Pubkey,
    pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WithdrawParams {
	pub program_id: Pubkey,
    /// Vault owner pubkey used for PDA derivation
    pub owner: Pubkey,
    /// SPL mint for collateral (USDT)
    pub mint: Pubkey,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyWithdrawParams {
    pub program_id: Pubkey,
    /// Authority that signs the instruction (owner or governance)
    pub authority: Pubkey,
    /// Vault owner used for PDA derivation
    pub owner: Pubkey,
    pub amount: u64,
}

pub fn build_instruction_initialize_vault(program_id: &Pubkey, user_pubkey: &Pubkey, usdt_mint: &Pubkey) -> AppResult<Instruction> {
    let (vault_pda, _) = derive_vault_pda(user_pubkey, program_id);
    let vault_ata = derive_associated_token_address(&vault_pda, usdt_mint);

    let accounts = vec![
        AccountMeta::new(*user_pubkey, true),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new(vault_ata, false),
        AccountMeta::new_readonly(*usdt_mint, false),
        AccountMeta::new_readonly(system_program::id(), false),
        AccountMeta::new_readonly(spl_token::id(), false),
        AccountMeta::new_readonly(spl_ata::id(), false),
        AccountMeta::new_readonly(sysvar::rent::id(), false),
    ];
    let data = anchor_discriminator("initialize_vault").to_vec();
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_deposit(params: &DepositParams) -> AppResult<Instruction> {
    let (vault_pda, _) = derive_vault_pda(&params.owner, &params.program_id);
    let user_ata = derive_associated_token_address(&params.owner, &params.mint);
    let vault_ata = derive_associated_token_address(&vault_pda, &params.mint);

    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new(user_ata, false),
        AccountMeta::new(vault_ata, false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ];
    let mut data = anchor_discriminator("deposit").to_vec();
    data.extend_from_slice(&params.amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_withdraw(params: &WithdrawParams) -> AppResult<Instruction> {
    let (vault_pda, _) = derive_vault_pda(&params.owner, &params.program_id);
    let user_ata = derive_associated_token_address(&params.owner, &params.mint);
    let vault_ata = derive_associated_token_address(&vault_pda, &params.mint);

    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new(vault_ata, false),
        AccountMeta::new(user_ata, false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ];
    let mut data = anchor_discriminator("withdraw").to_vec();
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
    let mut data = anchor_discriminator("withdraw").to_vec();
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

// -----------------
// Withdraw policy & requests (Anchor discriminators)
// -----------------

pub fn build_instruction_set_withdraw_min_delay(program_id: &Pubkey, owner: &Pubkey, seconds: i64) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(owner, program_id);
    let accounts = vec![
        AccountMeta::new(*owner, true),
        AccountMeta::new(vault_pda, false),
    ];
    let mut data = anchor_discriminator("set_withdraw_min_delay").to_vec();
    data.extend_from_slice(&seconds.to_le_bytes());
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_set_withdraw_rate_limit(program_id: &Pubkey, owner: &Pubkey, window_seconds: u32, max_amount: u64) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(owner, program_id);
    let accounts = vec![
        AccountMeta::new(*owner, true),
        AccountMeta::new(vault_pda, false),
    ];
    let mut data = anchor_discriminator("set_withdraw_rate_limit").to_vec();
    data.extend_from_slice(&window_seconds.to_le_bytes());
    data.extend_from_slice(&max_amount.to_le_bytes());
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_add_withdraw_whitelist(program_id: &Pubkey, owner: &Pubkey, address: &Pubkey) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(owner, program_id);
    let accounts = vec![
        AccountMeta::new(*owner, true),
        AccountMeta::new(vault_pda, false),
    ];
    let mut data = anchor_discriminator("add_withdraw_whitelist").to_vec();
    data.extend_from_slice(address.as_ref());
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_remove_withdraw_whitelist(program_id: &Pubkey, owner: &Pubkey, address: &Pubkey) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(owner, program_id);
    let accounts = vec![
        AccountMeta::new(*owner, true),
        AccountMeta::new(vault_pda, false),
    ];
    let mut data = anchor_discriminator("remove_withdraw_whitelist").to_vec();
    data.extend_from_slice(address.as_ref());
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_request_withdraw(program_id: &Pubkey, owner: &Pubkey, amount: u64) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(owner, program_id);
    let accounts = vec![
        AccountMeta::new(*owner, true),
        AccountMeta::new(vault_pda, false),
    ];
    let mut data = anchor_discriminator("request_withdraw").to_vec();
    data.extend_from_slice(&amount.to_le_bytes());
    Ok(Instruction { program_id: *program_id, accounts, data })
}

pub fn build_instruction_emergency_withdraw(params: &EmergencyWithdrawParams, mint: &Pubkey) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(&params.owner, &params.program_id);
    let (vault_authority, _abump) = derive_vault_authority_pda(&params.program_id);
    let vault_ata = derive_associated_token_address(&vault_pda, mint);
    let user_ata = derive_associated_token_address(&params.owner, mint);
    let accounts = vec![
        AccountMeta::new(params.authority, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new_readonly(vault_authority, false),
        AccountMeta::new(vault_ata, false),
        AccountMeta::new(user_ata, false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ];
    let mut data = anchor_discriminator("emergency_withdraw").to_vec();
    data.extend_from_slice(&params.amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldDepositParams {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub yield_program: Pubkey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldWithdrawParams {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub amount: u64,
    pub yield_program: Pubkey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompoundYieldParams {
    pub program_id: Pubkey,
    pub owner: Pubkey,
    pub compounded_amount: u64,
    pub yield_program: Pubkey,
}

pub fn build_instruction_yield_deposit(params: &YieldDepositParams) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(&params.owner, &params.program_id);
    let (va, _abump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new_readonly(va, false),
        AccountMeta::new_readonly(params.yield_program, false),
    ];
    let mut data = anchor_discriminator("yield_deposit").to_vec();
    data.extend_from_slice(&params.amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_yield_withdraw(params: &YieldWithdrawParams) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(&params.owner, &params.program_id);
    let (va, _abump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new_readonly(va, false),
        AccountMeta::new_readonly(params.yield_program, false),
    ];
    let mut data = anchor_discriminator("yield_withdraw").to_vec();
    data.extend_from_slice(&params.amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_compound_yield(params: &CompoundYieldParams) -> AppResult<Instruction> {
    let (vault_pda, _bump) = derive_vault_pda(&params.owner, &params.program_id);
    let (va, _abump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.owner, true),
        AccountMeta::new_readonly(params.owner, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new_readonly(va, false),
        AccountMeta::new_readonly(params.yield_program, false),
    ];
    let mut data = anchor_discriminator("compound_yield").to_vec();
    data.extend_from_slice(&params.compounded_amount.to_le_bytes());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

// -----------------
// Transfer collateral (internal settlements/liquidations)
// -----------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferCollateralParams {
    pub program_id: Pubkey,
    /// The allowlisted program id to pass as caller_program (e.g., position manager)
    pub caller_program: Pubkey,
    pub from_owner: Pubkey,
    pub to_owner: Pubkey,
    pub mint: Pubkey,
    pub amount: u64,
}

pub fn build_instruction_transfer_collateral(params: &TransferCollateralParams) -> AppResult<Instruction> {
    let (from_vault, _) = derive_vault_pda(&params.from_owner, &params.program_id);
    let (to_vault, _) = derive_vault_pda(&params.to_owner, &params.program_id);
    let from_vault_ata = derive_associated_token_address(&from_vault, &params.mint);
    let to_vault_ata = derive_associated_token_address(&to_vault, &params.mint);
    let (va, _) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new_readonly(params.caller_program, false),
        AccountMeta::new_readonly(va, false),
        AccountMeta::new_readonly(solana_program::sysvar::instructions::ID, false),
        AccountMeta::new(from_vault, false),
        AccountMeta::new(to_vault, false),
        AccountMeta::new(from_vault_ata, false),
        AccountMeta::new(to_vault_ata, false),
        AccountMeta::new_readonly(spl_token::id(), false),
    ];
    let mut data = anchor_discriminator("transfer_collateral").to_vec();
    data.extend_from_slice(&params.amount.to_le_bytes());
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

// Governance: yield whitelist/risk control (placeholders; program must align with these op codes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddYieldProgramParams { pub program_id: Pubkey, pub governance: Pubkey, pub yield_program: Pubkey }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveYieldProgramParams { pub program_id: Pubkey, pub governance: Pubkey, pub yield_program: Pubkey }
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetRiskLevelParams { pub program_id: Pubkey, pub governance: Pubkey, pub risk_level: u8 }

pub fn build_instruction_add_yield_program(params: &AddYieldProgramParams) -> AppResult<Instruction> {
    let (va, _bump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.governance, true),
        AccountMeta::new_readonly(va, false),
    ];
    let mut data = vec![51u8];
    data.extend_from_slice(params.yield_program.as_ref());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_remove_yield_program(params: &RemoveYieldProgramParams) -> AppResult<Instruction> {
    let (va, _bump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.governance, true),
        AccountMeta::new_readonly(va, false),
    ];
    let mut data = vec![52u8];
    data.extend_from_slice(params.yield_program.as_ref());
    Ok(Instruction { program_id: params.program_id, accounts, data })
}

pub fn build_instruction_set_risk_level(params: &SetRiskLevelParams) -> AppResult<Instruction> {
    let (va, _bump) = derive_vault_authority_pda(&params.program_id);
    let accounts = vec![
        AccountMeta::new(params.governance, true),
        AccountMeta::new_readonly(va, false),
    ];
    let data = vec![53u8, params.risk_level];
    Ok(Instruction { program_id: params.program_id, accounts, data })
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


