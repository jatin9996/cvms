# End-to-End Testing Guide

This directory contains comprehensive end-to-end tests for the Vault Management Service backend.

## Test Structure

### Test Files

1. **`test_utils.rs`** - Test utilities and helpers
2. **`e2e_vault_lifecycle.rs`** - Complete vault lifecycle tests (Initialize → Deposit → Lock/Unlock → Withdraw)
3. **`e2e_balance_tracker.rs`** - Balance tracking, reconciliation, and monitoring tests
4. **`e2e_transaction_builder.rs`** - Transaction building and SPL token handling tests
5. **`e2e_cpi_manager.rs`** - Cross-program integration (lock/unlock) tests
6. **`e2e_api_endpoints.rs`** - REST API endpoint integration tests
7. **`e2e_database_schema.rs`** - Database schema and data integrity tests
8. **`e2e_performance.rs`** - Performance and scalability tests
9. **`e2e_security.rs`** - Security and PDA derivation tests
10. **`e2e_vault_monitor.rs`** - Vault monitoring and analytics tests
11. **`e2e_websocket.rs`** - WebSocket real-time update tests

## Setup

### Prerequisites

1. **PostgreSQL Database**
   ```bash
   # Set test database URL
   export TEST_DATABASE_URL="postgresql://user:pass@localhost/test_db"
   ```

2. **Redis (Optional, for caching tests)**
   ```bash
   export TEST_REDIS_URL="redis://localhost:6379"
   ```

3. **Solana RPC (Optional, for on-chain tests)**
   ```bash
   export TEST_SOLANA_RPC_URL="https://api.devnet.solana.com"
   ```

### Running Tests

#### Run All Tests
```bash
cargo test --test '*'
```

#### Run Specific Test Suite
```bash
# Vault lifecycle tests
cargo test --test e2e_vault_lifecycle

# API endpoint tests
cargo test --test e2e_api_endpoints

# Performance tests
cargo test --test e2e_performance
```

#### Run Tests with Output
```bash
cargo test --test '*' -- --nocapture
```

## Test Coverage

### ✅ Core Components

- [x] Vault Manager
  - [x] Initialize vaults
  - [x] Process deposits
  - [x] Handle withdrawals
  - [x] Query balances
  - [x] Transaction history

- [x] Balance Tracker
  - [x] Calculate available balance
  - [x] Balance snapshots
  - [x] Reconciliation logging
  - [x] Low balance detection

- [x] Transaction Builder
  - [x] Build deposit transactions
  - [x] Build withdrawal transactions
  - [x] Compute budget instructions
  - [x] SPL token account handling

- [x] CPIManager
  - [x] Lock collateral
  - [x] Unlock collateral
  - [x] Lock/unlock consistency

- [x] Vault Monitor
  - [x] TVL calculation
  - [x] Unusual activity detection
  - [x] Analytics generation

### ✅ Database Schema

- [x] Vault accounts
- [x] Transaction history with status
- [x] Balance snapshots
- [x] Reconciliation logs
- [x] Audit trail
- [x] Transaction retry tracking

### ✅ REST API Endpoints

- [x] POST /vault/initialize
- [x] GET /vault/balance/:user
- [x] GET /vault/transactions/:user (with pagination)
- [x] GET /vault/tvl
- [x] GET /metrics

### ✅ WebSocket Streams

- [x] Balance updates
- [x] Deposit/withdrawal notifications
- [x] Lock/unlock events
- [x] TVL updates
- [x] Security alerts

### ✅ Security

- [x] Secure PDA derivation
- [x] Transaction idempotency
- [x] Audit trail completeness
- [x] Locked balance consistency

### ✅ Performance

- [x] Balance query < 50ms
- [x] Transaction history query performance
- [x] Concurrent balance queries
- [x] Large-scale vault operations (1000+ vaults)

## Test Scenarios

### Scenario 1: Complete Vault Lifecycle
```
1. Initialize vault
2. Deposit 10,000 tokens
3. Lock 5,000 tokens for position
4. Unlock 5,000 tokens
5. Withdraw 3,000 tokens
6. Verify final balance: 7,000
7. Verify transaction history
```

### Scenario 2: Multiple Operations
```
1. Create vault
2. Multiple deposits (1k, 2k, 3k)
3. Multiple withdrawals (500, 1k)
4. Verify balance: 4,500
5. Verify transaction count: 5
```

### Scenario 3: Balance Reconciliation
```
1. Set DB balance: 50,000
2. Set chain balance: 52,000
3. Run reconciliation
4. Verify discrepancy logged
5. Verify alert sent
```

### Scenario 4: Performance at Scale
```
1. Create 1,000 vaults
2. Query 100 vault balances concurrently
3. Verify all queries complete < 10 seconds
4. Verify individual queries < 50ms
```

## Continuous Integration

These tests are designed to run in CI/CD pipelines. Set the following environment variables:

```bash
TEST_DATABASE_URL=postgresql://test:test@localhost/test_db
TEST_REDIS_URL=redis://localhost:6379
TEST_SOLANA_RPC_URL=https://api.devnet.solana.com
```

## Notes

- Most tests are marked with `#[ignore]` because they require database/Redis connections
- Run with `cargo test --test '*' -- --ignored` to run ignored tests
- Tests clean up after themselves using `ctx.cleanup()`
- Performance tests verify requirements (< 50ms balance queries, < 2s deposits/withdrawals)

## Troubleshooting

### Database Connection Issues
- Ensure PostgreSQL is running
- Check `TEST_DATABASE_URL` is correct
- Verify database exists and is accessible

### Redis Connection Issues
- Redis is optional - tests will skip if unavailable
- Set `TEST_REDIS_URL` if you want to test caching

### Test Failures
- Check database state: `SELECT * FROM vaults;`
- Verify test cleanup ran: tables should be empty after tests
- Check logs for detailed error messages
