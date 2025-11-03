use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
	PgPoolOptions::new()
		.max_connections(5)
		.connect(database_url)
		.await
}

pub async fn init(pool: &PgPool) -> Result<(), sqlx::Error> {
	// Minimal tables to support nonces, tx history, and admin authorized programs
	sqlx::query(
		"CREATE TABLE IF NOT EXISTS nonces (
			nonce TEXT PRIMARY KEY,
			owner TEXT NOT NULL,
			issued_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
			used BOOLEAN NOT NULL DEFAULT FALSE
		)"
	)
	.execute(pool)
	.await?;

	sqlx::query(
		"CREATE TABLE IF NOT EXISTS transactions (
			id BIGSERIAL PRIMARY KEY,
			owner TEXT NOT NULL,
			signature TEXT NOT NULL,
			amount BIGINT,
			kind TEXT NOT NULL,
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)"
	)
	.execute(pool)
	.await?;

	// Idempotency on signature
	sqlx::query(
		"CREATE UNIQUE INDEX IF NOT EXISTS transactions_signature_key ON transactions(signature)"
	)
	.execute(pool)
	.await?;

	// Vault snapshots
	sqlx::query(
		"CREATE TABLE IF NOT EXISTS vaults (
			owner TEXT PRIMARY KEY,
			token_account TEXT,
			total_deposits BIGINT NOT NULL DEFAULT 0,
			total_withdrawals BIGINT NOT NULL DEFAULT 0,
			total_balance BIGINT NOT NULL DEFAULT 0,
			updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)"
	)
	.execute(pool)
	.await?;

	// Reconciliation logs
	sqlx::query(
		"CREATE TABLE IF NOT EXISTS reconciliation_logs (
			id BIGSERIAL PRIMARY KEY,
			vault_owner TEXT NOT NULL,
			token_account TEXT,
			db_balance BIGINT NOT NULL,
			chain_balance BIGINT NOT NULL,
			discrepancy BIGINT NOT NULL,
			threshold BIGINT NOT NULL,
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)"
	)
	.execute(pool)
	.await?;

	sqlx::query(
		"CREATE TABLE IF NOT EXISTS authorized_programs (
			program_id TEXT PRIMARY KEY,
			added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)"
	)
	.execute(pool)
	.await?;

	Ok(())
}

pub async fn insert_nonce(pool: &PgPool, nonce: &str, owner: &str) -> Result<(), sqlx::Error> {
	sqlx::query("INSERT INTO nonces (nonce, owner) VALUES ($1, $2) ON CONFLICT (nonce) DO NOTHING")
		.bind(nonce)
		.bind(owner)
		.execute(pool)
		.await?;
	Ok(())
}

pub async fn consume_nonce(pool: &PgPool, nonce: &str, owner: &str) -> Result<bool, sqlx::Error> {
	let res = sqlx::query("UPDATE nonces SET used = TRUE WHERE nonce = $1 AND owner = $2 AND used = FALSE")
		.bind(nonce)
		.bind(owner)
		.execute(pool)
		.await?;
	Ok(res.rows_affected() == 1)
}

pub async fn add_authorized_program(pool: &PgPool, program_id: &str) -> Result<(), sqlx::Error> {
	sqlx::query("INSERT INTO authorized_programs (program_id) VALUES ($1) ON CONFLICT (program_id) DO NOTHING")
		.bind(program_id)
		.execute(pool)
		.await?;
	Ok(())
}

pub async fn insert_transaction(
	pool: &PgPool,
	owner: &str,
	signature: &str,
	amount: Option<i64>,
	kind: &str,
) -> Result<(), sqlx::Error> {
	sqlx::query("INSERT INTO transactions (owner, signature, amount, kind) VALUES ($1, $2, $3, $4) ON CONFLICT (signature) DO NOTHING")
		.bind(owner)
		.bind(signature)
		.bind(amount)
		.bind(kind)
		.execute(pool)
		.await?;
	Ok(())
}

pub async fn list_transactions(pool: &PgPool, owner: &str, limit: i64, offset: i64) -> Result<Vec<(i64, String, Option<i64>, String, time::OffsetDateTime)>, sqlx::Error> {
	let rows = sqlx::query_as::<_, (i64, String, Option<i64>, String, time::OffsetDateTime)>(
		"SELECT id, signature, amount, kind, created_at FROM transactions WHERE owner = $1 ORDER BY id DESC LIMIT $2 OFFSET $3"
	)
	.bind(owner)
	.bind(limit)
	.bind(offset)
	.fetch_all(pool)
	.await?;
	Ok(rows)
}

pub async fn get_vault(pool: &PgPool, owner: &str) -> Result<Option<(Option<String>, i64)>, sqlx::Error> {
	let row = sqlx::query_as::<_, (Option<String>, i64)>(
		"SELECT token_account, total_balance FROM vaults WHERE owner = $1"
	)
	.bind(owner)
	.fetch_optional(pool)
	.await?;
	Ok(row)
}

pub async fn upsert_vault_token_account(pool: &PgPool, owner: &str, token_account: &str) -> Result<(), sqlx::Error> {
	sqlx::query(
		"INSERT INTO vaults (owner, token_account) VALUES ($1, $2)
		 ON CONFLICT (owner) DO UPDATE SET token_account = EXCLUDED.token_account"
	)
	.bind(owner)
	.bind(token_account)
	.execute(pool)
	.await?;
	Ok(())
}

pub async fn update_vault_snapshot(
	pool: &PgPool,
	owner: &str,
	new_total_balance: i64,
	deposit_delta: i64,
	withdraw_delta: i64,
) -> Result<(), sqlx::Error> {
	sqlx::query(
		"INSERT INTO vaults (owner, total_deposits, total_withdrawals, total_balance)
		 VALUES ($1, GREATEST($3,0), GREATEST($4,0), $2)
		 ON CONFLICT (owner) DO UPDATE SET
			 total_deposits = vaults.total_deposits + GREATEST($3,0),
			 total_withdrawals = vaults.total_withdrawals + GREATEST($4,0),
			 total_balance = $2,
			 updated_at = NOW()"
	)
	.bind(owner)
	.bind(new_total_balance)
	.bind(deposit_delta)
	.bind(withdraw_delta)
	.execute(pool)
	.await?;
	Ok(())
}

pub async fn list_vaults(pool: &PgPool) -> Result<Vec<(String, Option<String>, i64)>, sqlx::Error> {
	let rows = sqlx::query_as::<_, (String, Option<String>, i64)>(
		"SELECT owner, token_account, total_balance FROM vaults"
	)
	.fetch_all(pool)
	.await?;
	Ok(rows)
}

pub async fn insert_reconciliation_log(
	pool: &PgPool,
	vault_owner: &str,
	token_account: Option<&str>,
	db_balance: i64,
	chain_balance: i64,
	discrepancy: i64,
	threshold: i64,
) -> Result<(), sqlx::Error> {
	sqlx::query(
		"INSERT INTO reconciliation_logs (vault_owner, token_account, db_balance, chain_balance, discrepancy, threshold)
		 VALUES ($1, $2, $3, $4, $5, $6)"
	)
	.bind(vault_owner)
	.bind(token_account)
	.bind(db_balance)
	.bind(chain_balance)
	.bind(discrepancy)
	.bind(threshold)
	.execute(pool)
	.await?;
	Ok(())
}


