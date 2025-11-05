use crate::{api::AppState, db, protocols};
use tracing::info;

pub async fn run_yield_scheduler(state: AppState) {
    let interval = std::time::Duration::from_secs(600);
    loop {
        // Fetch and store protocol APYs
        let solend_apy = protocols::solend::fetch_usdt_apy().await.unwrap_or(0.0);
        let marginfi_apy = protocols::marginfi::fetch_usdt_apy().await.unwrap_or(0.0);
        let _ = db::insert_protocol_apy(&state.pool, "solend", solend_apy).await;
        let _ = db::insert_protocol_apy(&state.pool, "marginfi", marginfi_apy).await;

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


