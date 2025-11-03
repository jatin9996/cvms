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
	sqlx::query("INSERT INTO transactions (owner, signature, amount, kind) VALUES ($1, $2, $3, $4)")
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


