# CVMS Backend — Architecture

**Document version:** 1.0  
**Based on:** Current implementation (Rust, Axum, PostgreSQL, Solana RPC)  
**Purpose:** Client-facing system architecture for the Collateral Vault Management Service backend.

---

## 1. Overview

The **CVMS Backend** is a high-performance Rust service that sits between clients (wallets, UIs, position managers) and the **Collateral Vault** Solana program. It provides:

- **REST API** — Vault operations (initialize, deposit, withdraw), lock/unlock (CPI), admin and analytics
- **WebSocket API** — Real-time updates (deposits, withdrawals, balance, TVL, security alerts)
- **Transaction building & submission** — Instruction payloads for client-signed txs; server-signed txs for withdraw/lock/unlock/transfer
- **PostgreSQL** — Transaction history, nonces, vault snapshots, timelocks, multisig proposals, audit trail
- **Background tasks** — Event indexing, reconciliation, balance monitoring, timelock processing, yield tasks
- **Caching** — Optional Redis for balance and TVL to reduce RPC load
- **Security** — Wallet signature verification, nonce consumption, JWT for admin, 2FA (TOTP), rate limiting

The backend **does not** hold custody of funds; the Solana program does. The backend helps users build and submit transactions, indexes on-chain events, and maintains an off-chain view for history and analytics.

---

## 2. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           CLIENTS                                             │
├─────────────────────────────────────────────────────────────────────────────┤
│  Wallet / UI           │  Position Manager / Admin (JWT)                     │
│  - REST (deposit,      │  - REST (withdraw submit, lock/unlock,             │
│    withdraw, balance)  │    admin, internal transfer)                        │
│  - WebSocket (events)  │  - WebSocket (events)                               │
└────────────┬────────────┴────────────────────────┬──────────────────────────┘
             │                                     │
             ▼                                     ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                         CVMS BACKEND (this service)                           │
├─────────────────────────────────────────────────────────────────────────────┤
│  API Layer (Axum)                                                            │
│  • routes.rs — REST handlers (vault, admin, 2FA, analytics, PM lock/unlock)  │
│  • ws.rs — WebSocket handler (topic subscribe, account subscribe)           │
├─────────────────────────────────────────────────────────────────────────────┤
│  Core modules                                                                │
│  • auth — Wallet signature verification, admin JWT, 2FA (TOTP)                │
│  • vault — VaultManager (build init/deposit/withdraw ix, submit withdraw)   │
│  • cpi — CPIManager (submit lock/unlock via Position Manager)               │
│  • solana_client — RPC client, instruction builders, send tx                │
│  • db — PostgreSQL (nonces, transactions, vaults, timelocks, audit, etc.)   │
│  • cache — Redis (balance, TVL) — optional                                  │
│  • notify — Broadcast channels (deposit, withdraw, lock, unlock, balance…)   │
│  • metrics — Prometheus (deposits, withdrawals, balance latency, TVL)       │
├─────────────────────────────────────────────────────────────────────────────┤
│  Background tasks (tokio)                                                    │
│  • event_indexer — Logs subscribe → parse events → DB + notify               │
│  • reconciliation — DB vs chain balance; log discrepancy, notify            │
│  • monitor — Health / alerting                                               │
│  • timelocks — Cron for due timelocks                                        │
│  • yield_tasks — Yield protocol monitoring                                   │
│  • balance_monitor — Periodic balance check, low-balance alert              │
└───────────────────────────────┬─────────────────────────────────────────────┘
                                │
                                │  RPC / submit transaction
                                ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Solana RPC        │  Collateral Vault Program  │  Position Manager Program  │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Components

### 3.1 API Layer

| Component | Responsibility |
|-----------|----------------|
| **REST (routes.rs)** | Health, ready, auth/nonce; vault init/deposit/withdraw/schedule/emergency; balance, transactions, config, timelocks; multisig propose/approve; delegates; admin (whitelist, min-delay, rate-limit, vault authority, yield, risk); 2FA; PM lock/unlock; internal transfer; analytics (TVL series, distribution, utilization); metrics |
| **WebSocket (ws.rs)** | Upgrade at `/ws`; topic-based subscription (deposit_event, withdraw_event, lock_event, unlock_event, timelock_event, vault_balance_update, tvl_update, security_alert, analytics_update); optional account subscribe (Solana WS) |
| **AppState** | Shared state: PgPool, AppConfig, SolanaClient, Notifier, RateLimiter, optional Cache, Metrics |

### 3.2 Authentication & Security

| Mechanism | Use |
|-----------|-----|
| **Nonce** | Client requests nonce (`POST /auth/nonce`); signs message `action:params:nonce`; backend consumes nonce on use (deposit, withdraw, lock, unlock, etc.) to prevent replay |
| **Wallet signature** | Ed25519 verify on message (e.g. `deposit:{owner}:{amount}:{nonce}`); owner pubkey and signature in request body |
| **Admin JWT** | Bearer token; `verify_admin_jwt` for admin endpoints (emergency withdraw, internal transfer, vault authority, yield program, risk level, etc.) |
| **2FA (TOTP)** | Optional per-owner; required for withdraw when enabled; header `X-2FA-CODE` |
| **Rate limiting** | Governor layer on sensitive routes (e.g. withdraw, schedule-withdraw, pm/lock, pm/unlock) |

### 3.3 Vault & CPI

| Module | Responsibility |
|--------|----------------|
| **VaultManager (vault.rs)** | Build `initialize_vault`, `deposit`, `withdraw` instructions; submit withdraw tx (deployer as fee payer); query balance by owner (DB token_account → RPC or cache); available balance (DB total − locked) |
| **CPIManager (cpi.rs)** | Build and submit lock/unlock transactions that call **Position Manager** program; PM then CPIs into Collateral Vault; backend updates DB `locked_balance` (increment/decrement) after submit |

### 3.4 Database (PostgreSQL)

Key tables (from `db::init`):

| Table | Purpose |
|-------|---------|
| **nonces** | One-time nonces per owner; consumed on use |
| **transactions** | Signature, owner, amount, kind (deposit/withdraw/lock/unlock/…), status, retry_count |
| **vaults** | owner, token_account, total_deposits, total_withdrawals, total_balance, locked_balance, status |
| **timelocks** | owner, amount, unlock_at, status (scheduled/released) |
| **audit_trail** | owner, action, details (JSONB) |
| **reconciliation_logs** | DB vs chain balance discrepancy |
| **balance_snapshots** | Hourly/daily balance snapshots |
| **twofa** | owner, secret, enabled |
| **vault_delegates** | owner, delegate (off-chain allowlist) |
| **authorized_programs** | program_id (admin-managed) |
| **ms_proposals**, **ms_approvals**, **ms_signer_contacts** | Multisig withdraw flow |
| **withdraw_whitelist**, **yield_events**, **protocol_apy** | Policy and yield analytics |

### 3.5 Background Tasks

| Task | What it does |
|------|----------------|
| **event_indexer** | Subscribes to Solana logs (mentions program_id); parses deposit/withdraw/lock/unlock; upserts vault snapshot; inserts transaction; notifies via Notifier (deposit_tx, withdraw_tx, lock_tx, unlock_tx) |
| **reconciliation** | Periodically lists vaults; fetches chain balance per token_account; compares to DB; if discrepancy > threshold, logs and notifies vault_balance_tx |
| **monitor** | Health/alerting loop |
| **timelocks** | Cron: finds timelocks due within window; can trigger release or notify |
| **yield_tasks** | Yield protocol monitoring |
| **balance_monitor** | Periodic balance fetch per vault; detects changes and low balance; notifies vault_balance_tx |

### 3.6 Notifier (In-Memory Pub/Sub)

Broadcast channels used by routes and background tasks; WebSocket clients subscribe by topic:

- `deposit_tx`, `withdraw_tx`, `lock_tx`, `unlock_tx`
- `timelock_tx`, `vault_balance_tx`, `tvl_tx`, `security_tx`, `analytics_tx`

---

## 4. Configuration (Environment)

| Variable | Purpose |
|----------|---------|
| **HOST**, **PORT** | Bind address (default 0.0.0.0:8080) |
| **DATABASE_URL** | PostgreSQL connection string |
| **SOLANA_RPC_URL** | Solana RPC (and WS for indexer: https→wss) |
| **PROGRAM_ID** | Collateral Vault program ID |
| **USDT_MINT** | USDT mint pubkey |
| **DEPLOYER_KEYPAIR_PATH** | Keypair for fee payer and (where used) governance signer |
| **VAULT_AUTHORITY_PUBKEY** | Optional; validated for admin operations |
| **ADMIN_JWT_SECRET** | Secret for admin JWT |
| **POSITION_MANAGER_PROGRAM_ID** | Used by CPIManager for lock/unlock |
| **REDIS_URL** | Optional; if empty, cache disabled |
| **CACHE_TTL_SECONDS** | TTL for cached balance/TVL |
| **RECONCILIATION_THRESHOLD** | Discrepancy threshold (DB vs chain) |
| **LOW_BALANCE_THRESHOLD** | Alert when balance below this |
| **BALANCE_MONITOR_INTERVAL_SECONDS** | Balance monitor loop interval |

---

## 5. Data Flow Summary

- **Deposit:** Client gets nonce → signs `deposit:{owner}:{amount}:{nonce}` → POST deposit with signature → backend verifies signature, consumes nonce → returns **instruction payload** (client builds tx, signs, submits). Event indexer later sees log → DB + notify.
- **Withdraw:** Client signs withdraw message + optional 2FA → backend builds withdraw tx, **submits** with deployer as payer → records tx in DB, updates vault snapshot (best-effort), invalidates cache, notifies.
- **Lock/Unlock (PM):** Client signs pm_lock/pm_unlock message → backend (CPIManager) builds tx calling Position Manager → submits → updates DB locked_balance, notifies.
- **Balance:** GET balance by owner → resolve token_account from DB → RPC get_token_balance (or cache); metrics record latency.
- **WebSocket:** Client connects to `/ws`; sends `{ "topic": "deposit_event" }` (and optional `owner` filter) → backend spawns task that forwards Notifier messages to the socket.

---

## 6. Related Documents

- **FLOW.md** — Step-by-step flows (deposit, withdraw, lock/unlock, WebSocket, admin).
- **README.md** — Build, run, test, deploy.
- **BACKEND_REQUIREMENTS_COMPLIANCE.md** — Requirements vs implementation.
