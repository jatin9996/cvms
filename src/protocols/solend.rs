use anyhow::Result;

pub async fn fetch_usdt_apy() -> Result<f64> {
    // Solend mainnet reserves API
    let url = "https://api.solend.fi/v1/reserves?scope=mainnet";
    let res = reqwest::Client::new().get(url).send().await?;
    let v: serde_json::Value = res.json().await?;
    // Try to find USDT reserve and its supply APY (or deposit APY)
    if let Some(arr) = v.get("reserves").and_then(|x| x.as_array()) {
        for r in arr {
            let sym = r.get("symbol").and_then(|s| s.as_str()).unwrap_or("");
            if sym.eq_ignore_ascii_case("USDT") {
                // Prefer supplyApy, fallback to supplyInterest
                if let Some(apy) = r.get("supplyApy").and_then(|a| a.as_f64()) {
                    return Ok(apy);
                }
                if let Some(apy) = r.get("supplyInterest").and_then(|a| a.as_f64()) {
                    return Ok(apy);
                }
            }
        }
    }
    Ok(0.0)
}


