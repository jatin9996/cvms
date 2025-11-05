use crate::{api::AppState, db};
use tracing::info;

pub async fn run_yield_scheduler(state: AppState) {
    let interval = std::time::Duration::from_secs(600);
    loop {
        // Placeholder: fetch and store protocol APYs (use adapters in future)
        let _ = db::insert_protocol_apy(&state.pool, "solend", 0.0).await;
        let _ = db::insert_protocol_apy(&state.pool, "marginfi", 0.0).await;

        // Placeholder: iterate vaults and record a compound check event
        if let Ok(vaults) = db::list_vaults(&state.pool).await {
            for (owner, _token_acc, _bal) in vaults.into_iter() {
                let _ = db::insert_yield_event(&state.pool, &owner, "auto", 0, "compound_check").await;
            }
        }

        info!("yield scheduler tick");
        tokio::time::sleep(interval).await;
    }
}


