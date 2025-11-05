use anyhow::Result;

pub async fn fetch_usdt_apy() -> Result<f64> {
    // Public API subject to change; attempt, fallback to 0.0 on errors
    // Known endpoint variant (may change): https://api.marginfi.com/v1/pools
    let url = "https://api.marginfi.com/v1/pools";
    let res = reqwest::Client::new().get(url).send().await?;
    let v: serde_json::Value = res.json().await?;
    if let Some(arr) = v.as_array() {
        for p in arr {
            let sym = p.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
            if sym.eq_ignore_ascii_case("USDT") {
                if let Some(apy) = p.get("deposit_apy").and_then(|a| a.as_f64()) {
                    return Ok(apy);
                }
                if let Some(apy) = p.get("supplyApy").and_then(|a| a.as_f64()) {
                    return Ok(apy);
                }
            }
        }
    }
    Ok(0.0)
}


