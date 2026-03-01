# CVMS Backend — Flows

**Document version:** 1.0  
**Based on:** Current implementation  
**Purpose:** Client-facing description of main API and system flows.

---

## 1. Flow Summary

| Flow | Endpoint / Trigger | Actor | Backend role |
|------|--------------------|--------|--------------|
| Nonce | `POST /auth/nonce` | Client | Issue nonce, store in DB |
| Initialize vault | `POST /vault/initialize` | Client | Return instruction payload (client signs & submits) |
| Deposit | `POST /vault/deposit` | Client | Verify signature, consume nonce, return instruction payload |
| Withdraw | `POST /vault/withdraw` | Client | Verify signature + 2FA, consume nonce, **build & submit** tx, update DB, notify |
| Balance | `GET /vault/balance/:owner` | Client | Resolve token account, RPC (or cache), return balance |
| Lock (PM) | `POST /pm/lock` | Client | Verify signature, consume nonce, **submit** lock tx via PM, update DB locked_balance, notify |
| Unlock (PM) | `POST /pm/unlock` | Client | Verify signature, consume nonce, **submit** unlock tx via PM, update DB, notify |
| Schedule withdraw | `POST /vault/schedule-withdraw` | Client | Verify signature, consume nonce, build & submit schedule_timelock tx, insert timelock row, notify |
| Event indexer | Background | Service | Logs subscribe → parse events → DB + Notifier |
| Reconciliation | Background | Service | Compare DB vs chain balance, log discrepancy, notify |
| WebSocket | `GET /ws` | Client | Subscribe by topic; receive real-time events |

---

## 2. Authentication Flows

### 2.1 Nonce + Wallet Signature

Used for deposit, withdraw, lock, unlock, schedule-withdraw, request-withdraw, delegate add/remove, multisig propose/approve.

```
Client                                    Backend
   │                                         │
   │  POST /auth/nonce { "owner": "<pubkey>" }│
   │ ───────────────────────────────────────>│
   │  { "nonce": "<uuid>" }                   │  INSERT nonces (nonce, owner)
   │ <───────────────────────────────────────│
   │                                         │
   │  Sign message: "deposit:<owner>:<amount>:<nonce>"
   │  POST /vault/deposit { owner, amount, nonce, signature }
   │ ───────────────────────────────────────>│
   │                                         │  Verify signature
   │                                         │  UPDATE nonces SET used = TRUE WHERE nonce = ? AND owner = ?
   │                                         │  (if rows_affected != 1 → 400 invalid/used nonce)
   │  { "instruction": { ... } } or 4xx     │
   │ <───────────────────────────────────────│
```

### 2.2 Admin JWT

Used for emergency withdraw, internal transfer, vault authority add, yield program add/remove, risk level set, admin set vault token account.

```
Client sends: Authorization: Bearer <jwt>
Backend: verify_admin_jwt(token, ADMIN_JWT_SECRET); claims.role == "admin"
```

### 2.3 2FA (TOTP)

When 2FA is enabled for an owner, withdraw requires header `X-2FA-CODE` with current TOTP code. Backend verifies via `security::verify_totp(secret, code)`.

---

## 3. Vault Flows

### 3.1 Initialize Vault

**Request:** `POST /vault/initialize`  
**Body:** `{ "user_pubkey": "<pubkey>" }`  
**Response:** `{ "payload": { "program_id", "accounts", "data" } }` (base64-encoded instruction)

- Backend builds `initialize_vault` instruction (program_id, user, usdt_mint from config).
- Client uses payload to build transaction, **user signs**, client submits to Solana.
- No nonce/signature required for this endpoint in current implementation.

### 3.2 Deposit

**Request:** `POST /vault/deposit`  
**Body:** `{ "owner", "amount", "nonce", "signature" }`  
**Response:** `{ "instruction": { "program_id", "accounts", "data" } }`

1. Backend verifies signature on message `deposit:{owner}:{amount}:{nonce}`.
2. Consumes nonce (DB).
3. Builds deposit instruction; returns instruction payload.
4. Client builds transaction (deposit ix + optional compute budget), **owner signs**, client submits.
5. Later: event indexer sees log → inserts transaction, updates vault snapshot, notifies (deposit_tx).

### 3.3 Withdraw

**Request:** `POST /vault/withdraw`  
**Body:** `{ "owner", "amount", "nonce", "signature" }`  
**Headers:** Optional `X-2FA-CODE` if 2FA enabled for owner  
**Response:** `{ "signature": "<tx_signature>" }`

1. Backend verifies signature on `withdraw:{owner}:{amount}:{nonce}`.
2. Rate limiter check (per owner).
3. Consumes nonce.
4. If 2FA enabled for owner, verifies TOTP from header.
5. Builds withdraw instruction (+ compute budget); loads deployer keypair; builds and **submits** transaction (deployer = fee payer).
6. Inserts transaction (pending) in DB; best-effort vault snapshot update from chain; invalidates cache; notifies (withdraw_tx, vault_balance_tx).
7. Returns transaction signature.

### 3.4 Balance

**Request:** `GET /vault/balance/:owner`  
**Response:** `{ "balance": <u64>, "cached": true|false }`

- If Redis enabled: try cache by owner.
- Else: resolve owner → token_account from `vaults` table; if not found, treat owner as token account pubkey.
- RPC `get_token_balance`; cache result if Redis enabled.
- Metrics: balance query count and duration.

### 3.5 Transactions List

**Request:** `GET /vault/transactions/:owner?limit=50&offset=0`  
**Response:** `{ "items": [ { "id", "signature", "amount", "kind", "created_at" } ], "pagination": { "limit", "offset", "next" } }`

- Reads from `transactions` table for owner; ordered by id DESC; max 100 per page.

---

## 4. Position Manager (Lock / Unlock) Flows

### 4.1 Lock

**Request:** `POST /pm/lock`  
**Body:** `{ "owner", "amount", "nonce", "signature" }`  
**Response:** `{ "signature": "<tx_signature>" }`

1. Verify signature on `pm_lock:{owner}:{amount}:{nonce}`; consume nonce.
2. CPIManager builds transaction: compute budget + **Position Manager** `open_position(amount)` instruction (which CPIs into Collateral Vault `lock_collateral`).
3. Backend submits tx (deployer as payer).
4. Backend increments DB `locked_balance` for owner.
5. Inserts transaction (pending); audit log; notifies (lock_tx).

### 4.2 Unlock

**Request:** `POST /pm/unlock`  
**Body:** `{ "owner", "amount", "nonce", "signature" }`  
**Response:** `{ "signature": "<tx_signature>" }`

1. Verify signature on `pm_unlock:{owner}:{amount}:{nonce}`; consume nonce.
2. CPIManager builds transaction: Position Manager `close_position(amount)` (CPIs into Collateral Vault `unlock_collateral`).
3. Backend submits tx; decrements DB `locked_balance`; inserts transaction; notifies (unlock_tx).

---

## 5. Schedule Withdraw (Timelock) Flow

**Request:** `POST /vault/schedule-withdraw`  
**Body:** `{ "owner", "amount", "duration_seconds", "nonce", "signature" }`  
**Response:** `{ "signature", "unlock_at" }`

1. Verify signature on `schedule:{owner}:{amount}:{duration}:{nonce}`; consume nonce.
2. Build `schedule_timelock` instruction; submit tx (deployer payer).
3. Insert row in `timelocks` (owner, amount, unlock_at); notify (timelock_tx).

Client can later call `release_timelocks` (or backend can run timelock cron) to release matured timelocks on-chain.

---

## 6. Multisig Withdraw Flow

1. **Propose:** `POST /vault/propose-withdraw` — Body: owner, amount, threshold, signers (or empty to fetch from chain), nonce, signature. Creates `ms_proposals` row; optionally notifies signers (webhook).
2. **Approve:** `POST /vault/approve-withdraw` — Body: proposal_id, signer, nonce, signature. Inserts `ms_approvals`. If approvals >= threshold, backend builds **partial** withdraw transaction (multiple signers); returns `transaction_base64` and required signers for client-side co-signing.
3. **Status:** `GET /vault/proposal/:id` — Returns status, approvals count, threshold.

---

## 7. WebSocket Flow

**Connect:** `GET /ws` (WebSocket upgrade)

**Subscribe by topic:** Send JSON message, e.g.:

```json
{ "topic": "deposit_event" }
{ "topic": "withdraw_event", "owner": "<pubkey>" }
{ "topic": "vault_balance_update", "owner": "<pubkey>" }
{ "topic": "lock_event" }
{ "topic": "unlock_event" }
{ "topic": "timelock_event" }
{ "topic": "tvl_update" }
{ "topic": "security_alert" }
{ "topic": "analytics_update" }
```

- Backend subscribes to the corresponding Notifier channel and forwards messages to the WebSocket. Optional `owner` filters messages by string containment.
- **Account subscribe:** Send `{ "subscribe": "<pubkey>" }` — backend spawns Solana accountSubscribe for that pubkey and forwards account updates (binary length, slot) to the socket.

---

## 8. Event Indexer Flow (Background)

1. Connect to Solana WS (same URL as RPC, https→wss).
2. `logs_subscribe` with filter "mentions program_id".
3. For each log notification: parse logs to infer event kind (deposit, withdraw, lock, unlock, etc.) and owner/amount.
4. Update vault snapshot (e.g. from chain or parsed data); insert into `transactions`.
5. Send message to Notifier channel (deposit_tx, withdraw_tx, lock_tx, unlock_tx) so WebSocket clients receive it.

---

## 9. Reconciliation Flow (Background)

1. Periodically (e.g. every 60s) list vaults from DB (owner, token_account, total_balance).
2. For each vault with token_account, RPC `get_token_balance(token_account)`.
3. If |chain_balance - db_balance| > `reconciliation_threshold`: insert `reconciliation_logs` row; send to `vault_balance_tx` (notify clients).

---

## 10. Admin Flows (JWT Required)

| Endpoint | Purpose |
|----------|---------|
| `POST /vault/emergency-withdraw` | Build & submit emergency_withdraw (governance signer) |
| `POST /internal/transfer-collateral` | Build & submit transfer_collateral (from_owner, to_owner, amount; caller_program optional) |
| `POST /admin/vault-authority/add` | Add authorized program to DB |
| `POST /admin/yield-program/add` | Build & submit add_yield_program on Vault Authority |
| `POST /admin/yield-program/remove` | Build & submit remove_yield_program |
| `POST /admin/risk-level/set` | Build & submit set_risk_level |
| `POST /admin/vault-token-account/set` | Set vault token account in DB for owner; backfill snapshot |
| `POST /admin/withdraw/whitelist/add` | Return instruction payload for add withdraw whitelist |
| `DELETE /admin/withdraw/whitelist/remove` | Return instruction payload for remove |
| `POST /admin/withdraw/min-delay/set` | Return instruction payload for set min delay |
| `POST /admin/withdraw/rate-limit/set` | Return instruction payload for set rate limit |

---

## 11. Related Documents

- **ARCHITECTURE.md** — Components, DB, tasks, config.
- **README.md** — Build, run, test.
- **BACKEND_REQUIREMENTS_COMPLIANCE.md** — Requirements mapping.
