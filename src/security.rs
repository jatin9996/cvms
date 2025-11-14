use totp_rs::{Algorithm, TOTP};
use data_encoding::BASE32;

pub fn verify_totp(base32_secret: &str, code: &str) -> bool {
    let secret_bytes = match BASE32.decode(base32_secret.as_bytes()) { Ok(b) => b, Err(_) => return false };
    let totp = match TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes, None, String::new()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    totp.check_current(code).unwrap_or(false)
}


