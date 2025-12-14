use sqlx::{postgres::PgPoolOptions, PgPool};

pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    let max_connections = std::env::var("DB_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);
    PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .idle_timeout(std::time::Duration::from_secs(600))
        .max_lifetime(std::time::Duration::from_secs(1800))
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
		)",
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
			status TEXT NOT NULL DEFAULT 'pending',
			retry_count INT NOT NULL DEFAULT 0,
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
			updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Idempotency on signature
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS transactions_signature_key ON transactions(signature)",
    )
    .execute(pool)
    .await?;

    // Hot indexes for performance
    sqlx::query(
		"CREATE INDEX IF NOT EXISTS idx_transactions_owner_created_at ON transactions(owner, created_at DESC)"
	)
	.execute(pool)
	.await?;

    sqlx::query(
		"CREATE INDEX IF NOT EXISTS idx_transactions_status ON transactions(status)"
	)
	.execute(pool)
	.await?;

    // Add status column if missing (for existing databases)
    sqlx::query(
        "ALTER TABLE transactions ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'pending'"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "ALTER TABLE transactions ADD COLUMN IF NOT EXISTS retry_count INT NOT NULL DEFAULT 0"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "ALTER TABLE transactions ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()"
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
		)",
    )
    .execute(pool)
    .await?;

    // Timelock schedules per owner
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS timelocks (
			id BIGSERIAL PRIMARY KEY,
			owner TEXT NOT NULL,
			amount BIGINT NOT NULL,
			unlock_at TIMESTAMPTZ NOT NULL,
			status TEXT NOT NULL DEFAULT 'scheduled',
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
			updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_timelocks_owner_unlock_at ON timelocks(owner, unlock_at)",
    )
    .execute(pool)
    .await?;

    // Withdrawal whitelist (off-chain mirror for audit)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS withdraw_whitelist (
			owner TEXT NOT NULL,
			address TEXT NOT NULL,
			added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
			PRIMARY KEY (owner, address)
		)",
    )
    .execute(pool)
    .await?;

    // Two-factor auth secrets per owner
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS twofa (
			owner TEXT PRIMARY KEY,
			secret TEXT NOT NULL,
			enabled BOOLEAN NOT NULL DEFAULT FALSE,
			updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Add new columns if missing
    sqlx::query(
        "ALTER TABLE vaults ADD COLUMN IF NOT EXISTS locked_balance BIGINT NOT NULL DEFAULT 0",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "ALTER TABLE vaults ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active'",
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
		)",
    )
    .execute(pool)
    .await?;

    // Balance snapshots (hourly/daily)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS balance_snapshots (
			id BIGSERIAL PRIMARY KEY,
			owner TEXT NOT NULL,
			balance BIGINT NOT NULL,
			locked_balance BIGINT NOT NULL,
			granularity TEXT NOT NULL,
			recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Audit trail
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS audit_trail (
			id BIGSERIAL PRIMARY KEY,
			owner TEXT,
			action TEXT NOT NULL,
			details JSONB,
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Yield events/history
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS yield_events (
			id BIGSERIAL PRIMARY KEY,
			owner TEXT NOT NULL,
			protocol TEXT NOT NULL,
			amount BIGINT NOT NULL,
			kind TEXT NOT NULL,
			created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Protocol APY snapshots (for analytics)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS protocol_apy (
			id BIGSERIAL PRIMARY KEY,
			protocol TEXT NOT NULL,
			apy DOUBLE PRECISION NOT NULL,
			recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Vault delegates (off-chain allowlist for UI and prechecks)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS vault_delegates (
			owner TEXT NOT NULL,
			delegate TEXT NOT NULL,
			added_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
			PRIMARY KEY (owner, delegate)
		)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS authorized_programs (
			program_id TEXT PRIMARY KEY,
			added_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
		)",
    )
    .execute(pool)
    .await?;

    // Multisig proposal tables
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ms_proposals (
            id UUID PRIMARY KEY,
            owner TEXT NOT NULL,
            amount BIGINT NOT NULL,
            threshold INT NOT NULL,
            signers JSONB NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ms_approvals (
            id BIGSERIAL PRIMARY KEY,
            proposal_id UUID NOT NULL REFERENCES ms_proposals(id) ON DELETE CASCADE,
            signer TEXT NOT NULL,
            signature TEXT NOT NULL,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            UNIQUE (proposal_id, signer)
        )",
    )
    .execute(pool)
    .await?;

    // Optional contacts for signers to deliver notifications
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ms_signer_contacts (
            pubkey TEXT PRIMARY KEY,
            email TEXT,
            webhook TEXT,
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )",
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
    let res = sqlx::query(
        "UPDATE nonces SET used = TRUE WHERE nonce = $1 AND owner = $2 AND used = FALSE",
    )
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
    insert_transaction_with_status(pool, owner, signature, amount, kind, "pending").await
}

pub async fn insert_transaction_with_status(
    pool: &PgPool,
    owner: &str,
    signature: &str,
    amount: Option<i64>,
    kind: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO transactions (owner, signature, amount, kind, status) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (signature) DO NOTHING")
		.bind(owner)
		.bind(signature)
		.bind(amount)
		.bind(kind)
		.bind(status)
		.execute(pool)
		.await?;
    Ok(())
}

pub async fn update_transaction_status(
    pool: &PgPool,
    signature: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE transactions SET status = $1, updated_at = NOW() WHERE signature = $2")
        .bind(status)
        .bind(signature)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn increment_transaction_retry(
    pool: &PgPool,
    signature: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE transactions SET retry_count = retry_count + 1, updated_at = NOW() WHERE signature = $1")
        .bind(signature)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_pending_transactions(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<(String, String, String, i32)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, String, String, i32)>(
        "SELECT owner, signature, kind, retry_count FROM transactions WHERE status = 'pending' ORDER BY created_at ASC LIMIT $1"
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_transactions(
    pool: &PgPool,
    owner: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<(i64, String, Option<i64>, String, time::OffsetDateTime)>, sqlx::Error> {
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

pub async fn get_vault(
    pool: &PgPool,
    owner: &str,
) -> Result<Option<(Option<String>, i64)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT token_account, total_balance FROM vaults WHERE owner = $1",
    )
    .bind(owner)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn upsert_vault_token_account(
    pool: &PgPool,
    owner: &str,
    token_account: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO vaults (owner, token_account) VALUES ($1, $2)
		 ON CONFLICT (owner) DO UPDATE SET token_account = EXCLUDED.token_account",
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
			 updated_at = NOW()",
    )
    .bind(owner)
    .bind(new_total_balance)
    .bind(deposit_delta)
    .bind(withdraw_delta)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_locked_balance(pool: &PgPool, owner: &str) -> Result<i64, sqlx::Error> {
    let row = sqlx::query_scalar::<_, i64>("SELECT locked_balance FROM vaults WHERE owner = $1")
        .bind(owner)
        .fetch_optional(pool)
        .await?;
    Ok(row.unwrap_or(0))
}

pub async fn increment_locked_balance(
    pool: &PgPool,
    owner: &str,
    delta: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
		"INSERT INTO vaults (owner, locked_balance)
		 VALUES ($1, GREATEST($2,0))
		 ON CONFLICT (owner) DO UPDATE SET locked_balance = GREATEST(vaults.locked_balance + $2, 0), updated_at = NOW()"
	)
	.bind(owner)
	.bind(delta)
	.execute(pool)
	.await?;
    Ok(())
}

pub async fn insert_balance_snapshot(
    pool: &PgPool,
    owner: &str,
    balance: i64,
    locked: i64,
    granularity: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO balance_snapshots (owner, balance, locked_balance, granularity) VALUES ($1, $2, $3, $4)")
		.bind(owner)
		.bind(balance)
		.bind(locked)
		.bind(granularity)
		.execute(pool)
		.await?;
    Ok(())
}

pub async fn insert_audit_log(
    pool: &PgPool,
    owner: Option<&str>,
    action: &str,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO audit_trail (owner, action, details) VALUES ($1, $2, $3)")
        .bind(owner)
        .bind(action)
        .bind(details)
        .execute(pool)
        .await?;
    Ok(())
}

// -----------------
// Whitelist helpers
// -----------------
pub async fn whitelist_add(pool: &PgPool, owner: &str, address: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO withdraw_whitelist (owner, address) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(owner)
    .bind(address)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn whitelist_remove(
    pool: &PgPool,
    owner: &str,
    address: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM withdraw_whitelist WHERE owner = $1 AND address = $2")
        .bind(owner)
        .bind(address)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn whitelist_list(pool: &PgPool, owner: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT address FROM withdraw_whitelist WHERE owner = $1 ORDER BY added_at DESC",
    )
    .bind(owner)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(a,)| a).collect())
}

// -----------------
// 2FA helpers
// -----------------
pub async fn twofa_upsert(
    pool: &PgPool,
    owner: &str,
    secret: &str,
    enabled: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO twofa (owner, secret, enabled) VALUES ($1, $2, $3)
         ON CONFLICT (owner) DO UPDATE SET secret = EXCLUDED.secret, enabled = EXCLUDED.enabled, updated_at = NOW()"
    )
    .bind(owner)
    .bind(secret)
    .bind(enabled)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn twofa_get(pool: &PgPool, owner: &str) -> Result<Option<(String, bool)>, sqlx::Error> {
    let row =
        sqlx::query_as::<_, (String, bool)>("SELECT secret, enabled FROM twofa WHERE owner = $1")
            .bind(owner)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

pub async fn insert_yield_event(
    pool: &PgPool,
    owner: &str,
    protocol: &str,
    amount: i64,
    kind: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO yield_events (owner, protocol, amount, kind) VALUES ($1, $2, $3, $4)")
        .bind(owner)
        .bind(protocol)
        .bind(amount)
        .bind(kind)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn insert_protocol_apy(
    pool: &PgPool,
    protocol: &str,
    apy: f64,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO protocol_apy (protocol, apy) VALUES ($1, $2)")
        .bind(protocol)
        .bind(apy)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn latest_protocol_apys(pool: &PgPool) -> Result<Vec<(String, f64)>, sqlx::Error> {
    // Select latest APY per protocol
    let rows = sqlx::query_as::<_, (String, f64)>(
        "SELECT DISTINCT ON (protocol) protocol, apy FROM protocol_apy ORDER BY protocol, recorded_at DESC"
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// -----------------
// Timelocks
// -----------------
pub async fn timelock_insert(
    pool: &PgPool,
    owner: &str,
    amount: i64,
    unlock_at: time::OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO timelocks (owner, amount, unlock_at) VALUES ($1, $2, $3)")
        .bind(owner)
        .bind(amount)
        .bind(unlock_at)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn timelock_list(
    pool: &PgPool,
    owner: &str,
) -> Result<Vec<(i64, i64, time::OffsetDateTime, String)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, i64, time::OffsetDateTime, String)>(
        "SELECT id, amount, unlock_at, status FROM timelocks WHERE owner = $1 ORDER BY unlock_at ASC"
    )
    .bind(owner)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn timelock_mark_status(pool: &PgPool, id: i64, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE timelocks SET status = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn timelock_due_within(
    pool: &PgPool,
    seconds: i64,
) -> Result<Vec<(i64, String, i64, time::OffsetDateTime)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (i64, String, i64, time::OffsetDateTime)>(
        "SELECT id, owner, amount, unlock_at FROM timelocks WHERE status = 'scheduled' AND unlock_at <= NOW() + make_interval(secs => $1) ORDER BY unlock_at ASC"
    )
    .bind(seconds)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// -----------------
// Delegates (off-chain)
// -----------------
pub async fn delegate_add(pool: &PgPool, owner: &str, delegate: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO vault_delegates (owner, delegate) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(owner)
    .bind(delegate)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn delegate_remove(
    pool: &PgPool,
    owner: &str,
    delegate: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query("DELETE FROM vault_delegates WHERE owner = $1 AND delegate = $2")
        .bind(owner)
        .bind(delegate)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn delegate_list(pool: &PgPool, owner: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT delegate FROM vault_delegates WHERE owner = $1 ORDER BY added_at DESC",
    )
    .bind(owner)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(d,)| d).collect())
}

// -----------------
// Multisig proposals
// -----------------
pub async fn ms_create_proposal(
    pool: &PgPool,
    id: &str,
    owner: &str,
    amount: i64,
    threshold: i32,
    signers_json: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ms_proposals (id, owner, amount, threshold, signers) VALUES ($1, $2, $3, $4, $5)"
    )
    .bind(id)
    .bind(owner)
    .bind(amount)
    .bind(threshold)
    .bind(signers_json)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn ms_insert_approval(
    pool: &PgPool,
    proposal_id: &str,
    signer: &str,
    signature: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO ms_approvals (proposal_id, signer, signature) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING"
    )
    .bind(proposal_id)
    .bind(signer)
    .bind(signature)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() == 1)
}

pub async fn ms_get_proposal(
    pool: &PgPool,
    id: &str,
) -> Result<Option<(String, i64, i32, serde_json::Value, String)>, sqlx::Error> {
    let row = sqlx::query_as::<_, (String, i64, i32, serde_json::Value, String)>(
        "SELECT owner, amount, threshold, signers, status FROM ms_proposals WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn ms_count_approvals(pool: &PgPool, id: &str) -> Result<i64, sqlx::Error> {
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM ms_approvals WHERE proposal_id = $1")
            .bind(id)
            .fetch_one(pool)
            .await?;
    Ok(count)
}

pub async fn ms_set_status(pool: &PgPool, id: &str, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE ms_proposals SET status = $2, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .bind(status)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn ms_list_approvals(pool: &PgPool, id: &str) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT signer FROM ms_approvals WHERE proposal_id = $1 ORDER BY id ASC",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

pub async fn ms_get_contacts_for(
    pool: &PgPool,
    pubkeys: &[String],
) -> Result<Vec<(String, Option<String>, Option<String>)>, sqlx::Error> {
    if pubkeys.is_empty() {
        return Ok(vec![]);
    }
    // Build simple IN clause
    let query = String::from(
        "SELECT pubkey, email, webhook FROM ms_signer_contacts WHERE pubkey = ANY($1)",
    );
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(&query)
        .bind(pubkeys)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn list_vaults(pool: &PgPool) -> Result<Vec<(String, Option<String>, i64)>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, Option<String>, i64)>(
        "SELECT owner, token_account, total_balance FROM vaults",
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
