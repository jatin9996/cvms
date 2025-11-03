## CVMS Backend

Minimal Axum server scaffold for Solana vault management backend.

### Prerequisites

- Rust (stable). On Windows, install rustup and Visual Studio Build Tools with the Desktop C++ workload.

### Environment Variables

- HOST (default: 0.0.0.0)
- PORT (default: 8080)
- DATABASE_URL
- SOLANA_RPC_URL
- PROGRAM_ID
- USDT_MINT
- DEPLOYER_KEYPAIR_PATH
- VAULT_AUTHORITY_PUBKEY

### Run

```bash
cargo run
```

Routes:
- GET /health → { "status": "ok" }

### Next Steps

- Add Solana client layer (src/solana_client.rs)
- Add SQLx and Postgres integration
- Implement REST + WebSocket APIs

