use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
	pub host: String,
	pub port: u16,
	pub database_url: String,
	pub solana_rpc_url: String,
	pub program_id: String,
	pub usdt_mint: String,
	pub deployer_keypair_path: String,
	pub vault_authority_pubkey: String,
	pub admin_jwt_secret: String,
	pub position_manager_program_id: String,
	pub reconciliation_threshold: i64,
	pub low_balance_threshold: i64,
}

impl AppConfig {
	pub fn from_env() -> Self {
		Self {
			host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
			port: std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080),
			database_url: std::env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
			solana_rpc_url: std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| "https://api.devnet.solana.com".to_string()),
			program_id: std::env::var("PROGRAM_ID").unwrap_or_default(),
			usdt_mint: std::env::var("USDT_MINT").unwrap_or_default(),
			deployer_keypair_path: std::env::var("DEPLOYER_KEYPAIR_PATH").unwrap_or_default(),
			vault_authority_pubkey: std::env::var("VAULT_AUTHORITY_PUBKEY").unwrap_or_default(),
			admin_jwt_secret: std::env::var("ADMIN_JWT_SECRET").unwrap_or_default(),
			position_manager_program_id: std::env::var("POSITION_MANAGER_PROGRAM_ID").unwrap_or_default(),
			reconciliation_threshold: std::env::var("RECONCILIATION_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
			low_balance_threshold: std::env::var("LOW_BALANCE_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0),
		}
	}
}


