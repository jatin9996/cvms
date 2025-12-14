use prometheus::{
    register_counter, register_gauge, register_histogram, Counter, Gauge, Histogram, HistogramOpts,
    Opts, Registry,
};
use std::sync::Arc;

pub struct Metrics {
    pub vault_operations: Counter,
    pub vault_deposits: Counter,
    pub vault_withdrawals: Counter,
    pub vault_locks: Counter,
    pub vault_unlocks: Counter,
    pub vault_balance_queries: Counter,
    pub transaction_submissions: Counter,
    pub transaction_failures: Counter,
    pub reconciliation_discrepancies: Counter,
    pub active_vaults: Gauge,
    pub total_value_locked: Gauge,
    pub request_duration: Histogram,
    pub balance_query_duration: Histogram,
    pub transaction_duration: Histogram,
    pub registry: Registry,
}

impl Metrics {
    pub fn new() -> Result<Arc<Self>, prometheus::Error> {
        let registry = Registry::new();

        let vault_operations = register_counter!(Opts::new(
            "vault_operations_total",
            "Total number of vault operations"
        ))?;
        registry.register(Box::new(vault_operations.clone()))?;

        let vault_deposits = register_counter!(Opts::new(
            "vault_deposits_total",
            "Total number of deposits"
        ))?;
        registry.register(Box::new(vault_deposits.clone()))?;

        let vault_withdrawals = register_counter!(Opts::new(
            "vault_withdrawals_total",
            "Total number of withdrawals"
        ))?;
        registry.register(Box::new(vault_withdrawals.clone()))?;

        let vault_locks = register_counter!(Opts::new(
            "vault_locks_total",
            "Total number of locks"
        ))?;
        registry.register(Box::new(vault_locks.clone()))?;

        let vault_unlocks = register_counter!(Opts::new(
            "vault_unlocks_total",
            "Total number of unlocks"
        ))?;
        registry.register(Box::new(vault_unlocks.clone()))?;

        let vault_balance_queries = register_counter!(Opts::new(
            "vault_balance_queries_total",
            "Total number of balance queries"
        ))?;
        registry.register(Box::new(vault_balance_queries.clone()))?;

        let transaction_submissions = register_counter!(Opts::new(
            "transaction_submissions_total",
            "Total number of transaction submissions"
        ))?;
        registry.register(Box::new(transaction_submissions.clone()))?;

        let transaction_failures = register_counter!(Opts::new(
            "transaction_failures_total",
            "Total number of transaction failures"
        ))?;
        registry.register(Box::new(transaction_failures.clone()))?;

        let reconciliation_discrepancies = register_counter!(Opts::new(
            "reconciliation_discrepancies_total",
            "Total number of reconciliation discrepancies"
        ))?;
        registry.register(Box::new(reconciliation_discrepancies.clone()))?;

        let active_vaults = register_gauge!(Opts::new(
            "active_vaults",
            "Number of active vaults"
        ))?;
        registry.register(Box::new(active_vaults.clone()))?;

        let total_value_locked = register_gauge!(Opts::new(
            "total_value_locked",
            "Total value locked in vaults"
        ))?;
        registry.register(Box::new(total_value_locked.clone()))?;

        let request_duration_opts = HistogramOpts::new(
            "request_duration_seconds",
            "Request duration in seconds",
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0]);
        let request_duration = register_histogram!(request_duration_opts)?;
        registry.register(Box::new(request_duration.clone()))?;

        let balance_query_duration_opts = HistogramOpts::new(
            "balance_query_duration_seconds",
            "Balance query duration in seconds",
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5]);
        let balance_query_duration = register_histogram!(balance_query_duration_opts)?;
        registry.register(Box::new(balance_query_duration.clone()))?;

        let transaction_duration_opts = HistogramOpts::new(
            "transaction_duration_seconds",
            "Transaction duration in seconds",
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0]);
        let transaction_duration = register_histogram!(transaction_duration_opts)?;
        registry.register(Box::new(transaction_duration.clone()))?;

        Ok(Arc::new(Self {
            vault_operations,
            vault_deposits,
            vault_withdrawals,
            vault_locks,
            vault_unlocks,
            vault_balance_queries,
            transaction_submissions,
            transaction_failures,
            reconciliation_discrepancies,
            active_vaults,
            total_value_locked,
            request_duration,
            balance_query_duration,
            transaction_duration,
            registry,
        }))
    }
}

// Default implementation removed - use Metrics::new() which returns Arc<Metrics>
// This is more appropriate since Metrics is always used behind Arc
