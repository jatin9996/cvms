# CVMS Backend - Collateral Vault Management Service

A high-performance Rust backend service for managing collateral vaults on Solana, providing REST API, WebSocket support, and background task processing.

## Overview

The CVMS Backend provides:
- **REST API**: Comprehensive API for vault operations
- **WebSocket API**: Real-time updates and notifications
- **Background Tasks**: Event indexing, reconciliation, monitoring
- **Database Management**: PostgreSQL for state tracking
- **Caching**: Redis integration for performance
- **Metrics**: Prometheus metrics for monitoring

## Features

- ✅ **REST API**: Full CRUD operations for vaults
- ✅ **WebSocket**: Real-time balance and transaction updates
- ✅ **Authentication**: Wallet signature verification, JWT, 2FA
- ✅ **Rate Limiting**: Per-user and global rate limits
- ✅ **Caching**: Redis caching for frequently accessed data
- ✅ **Metrics**: Prometheus metrics endpoint
- ✅ **Background Tasks**: Event indexing, reconciliation, monitoring
- ✅ **Security**: Comprehensive security controls

## Quick Start

### Prerequisites

- **Rust**: Latest stable version
- **PostgreSQL**: 14+
- **Redis**: 6+ (optional, for caching)
- **Solana RPC**: Access to Solana RPC endpoint

### Installation

```bash
# Install dependencies
cargo build

# Run migrations (auto-initialized on startup)
# Database tables are created automatically

# Run the server
cargo run
```

### Configuration

Create a `.env` file:

```env
HOST=0.0.0.0
PORT=8080
DATABASE_URL=postgresql://user:password@localhost:5432/cvmsback
SOLANA_RPC_URL=https://api.testnet.solana.com
PROGRAM_ID=5qgA2qcz6zXYiJJkomV1LJv8UhKueyNsqeCWJd6jC9pT
USDT_MINT=4QHVBbG3H8kbwvcSwPnze3sC91kdeYWxNf8S5hkZ9nbZ
DEPLOYER_KEYPAIR_PATH=/path/to/keypair.json
ADMIN_JWT_SECRET=your-secret-here
REDIS_URL=redis://localhost:6379
```

## Project Structure

```
cvmsback/
├── src/
│   ├── main.rs              # Application entry point
│   ├── lib.rs               # Library root
│   ├── api/                 # API layer
│   │   ├── routes.rs        # REST endpoints
│   │   ├── ws.rs           # WebSocket server
│   │   └── mod.rs
│   ├── db.rs                # Database operations
│   ├── solana_client.rs     # Solana RPC client
│   ├── cpi.rs               # CPI manager
│   ├── auth.rs              # Authentication
│   ├── security.rs          # Security utilities
│   ├── cache.rs             # Redis caching
│   ├── metrics.rs           # Prometheus metrics
│   ├── tasks/               # Background tasks
│   │   ├── event_indexer.rs
│   │   ├── reconciliation.rs
│   │   ├── monitor.rs
│   │   └── ...
│   └── protocols/           # Yield protocol integrations
├── tests/                   # Test suites
├── scripts/                 # Utility scripts
├── docs/                    # Documentation
│   ├── ARCHITECTURE.md      # System architecture (client-facing)
│   ├── FLOW.md              # API & background flows (client-facing)
│   └── README.md            # Docs index
└── Cargo.toml
```

## Documentation

**Client-facing (architecture & flows):**

- **[Architecture](./docs/ARCHITECTURE.md)**: System overview, API layer, auth, Vault/CPI managers, database, background tasks, config
- **[Flow](./docs/FLOW.md)**: Step-by-step flows (nonce + signature, deposit/withdraw, lock/unlock, WebSocket, admin)

**Other:**

- **[API Documentation](./docs/API_DOCUMENTATION.md)**: Complete API reference (if present)

### Key Endpoints

- `GET /health` - Health check
- `GET /ready` - Readiness check
- `GET /metrics` - Prometheus metrics
- `POST /api/vault/initialize` - Initialize vault
- `POST /api/vault/deposit` - Deposit instruction
- `POST /api/vault/withdraw` - Withdraw (backend submits)
- `GET /api/vault/balance/:owner` - Get balance
- `WS /ws` - WebSocket connection

## Development

### Running

```bash
# Development mode
cargo run

# Release mode
cargo build --release
./target/release/cvmsback
```

### Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with output
cargo test -- --nocapture
```

### Code Quality

```bash
# Format code
cargo fmt

# Lint
cargo clippy

# Check
cargo check
```

## Background Tasks

The backend runs several background tasks:

1. **Event Indexer**: Monitors on-chain events
2. **Reconciliation**: Compares on-chain vs database state
3. **Monitor**: Health checks and alerting
4. **Timelock Cron**: Processes scheduled timelocks
5. **Yield Tasks**: Yield protocol monitoring
6. **Balance Monitor**: Periodic balance checks

## Database Schema

The database automatically initializes tables on startup. Key tables:

- `vaults` - Vault state
- `transactions` - Transaction history
- `audit_logs` - Audit trail
- `nonces` - Nonce management
- `timelocks` - Timelock tracking
- `multisig_proposals` - Multisig proposals
- And more...

## Monitoring

### Metrics

Prometheus metrics available at `/metrics`:

- `vault_deposits_total` - Total deposits
- `vault_withdrawals_total` - Total withdrawals
- `vault_operations_total` - Total operations
- `transaction_submissions_total` - Transaction submissions
- `balance_query_duration_seconds` - Balance query duration
- `total_value_locked` - TVL gauge

### Health Checks

- `GET /health` - Basic health check
- `GET /ready` - Database and RPC connectivity

## Security

- ✅ Wallet signature verification
- ✅ JWT authentication for admin
- ✅ 2FA support (TOTP)
- ✅ Rate limiting
- ✅ Input validation
- ✅ SQL injection prevention
- ✅ Audit logging

## Performance

- **Target**: 10,000+ vaults
- **Response Time**: < 200ms (P95)
- **Throughput**: 100+ RPS
- **Caching**: Redis for frequently accessed data

## Deployment

See deployment instructions in the main documentation.

### Docker

```bash
docker build -t cvmsback .
docker run -d --env-file .env -p 8080:8080 cvmsback
```

### Systemd

See deployment guide for systemd service configuration.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Add tests
5. Submit a pull request

## License

ISC

## Support

For issues, questions, or contributions, please open an issue on the repository.

## Related Projects

- **[Smart Contract](../cvms/collateral-vault/): Solana program**
- **[Documentation](./docs/): [ARCHITECTURE](./docs/ARCHITECTURE.md), [FLOW](./docs/FLOW.md), API reference**
