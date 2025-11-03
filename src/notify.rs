use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct Notifier {
	pub deposit_tx: broadcast::Sender<String>,
	pub withdraw_tx: broadcast::Sender<String>,
	pub lock_tx: broadcast::Sender<String>,
	pub unlock_tx: broadcast::Sender<String>,
	pub vault_balance_tx: broadcast::Sender<String>,
	pub tvl_tx: broadcast::Sender<String>,
}

impl Notifier {
	pub fn new(capacity: usize) -> Arc<Self> {
		let (deposit_tx, _) = broadcast::channel(capacity);
		let (withdraw_tx, _) = broadcast::channel(capacity);
		let (lock_tx, _) = broadcast::channel(capacity);
		let (unlock_tx, _) = broadcast::channel(capacity);
		let (vault_balance_tx, _) = broadcast::channel(capacity);
		let (tvl_tx, _) = broadcast::channel(capacity);
		Arc::new(Self { deposit_tx, withdraw_tx, lock_tx, unlock_tx, vault_balance_tx, tvl_tx })
	}
}


