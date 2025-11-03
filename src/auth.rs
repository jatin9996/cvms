use crate::error::{AppError, AppResult};
use ed25519_dalek::{PublicKey, Signature, Verifier};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

pub fn verify_wallet_signature(owner_pubkey_base58: &str, message: &[u8], signature_base58: &str) -> AppResult<()> {
	let pk_bytes = bs58::decode(owner_pubkey_base58)
		.into_vec()
		.map_err(|_| AppError::BadRequest("invalid owner pubkey".to_string()))?;
	let sig_bytes = bs58::decode(signature_base58)
		.into_vec()
		.map_err(|_| AppError::BadRequest("invalid signature".to_string()))?;
	let public_key = PublicKey::from_bytes(&pk_bytes).map_err(|_| AppError::BadRequest("invalid pubkey bytes".to_string()))?;
	let signature = Signature::from_bytes(&sig_bytes).map_err(|_| AppError::BadRequest("invalid signature bytes".to_string()))?;
	public_key
		.verify(message, &signature)
		.map_err(|_| AppError::Unauthorized)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminClaims {
	pub sub: String,
	pub role: String,
	pub exp: usize,
}

pub fn verify_admin_jwt(token: &str, secret: &str) -> AppResult<AdminClaims> {
	let mut validation = Validation::new(Algorithm::HS256);
	validation.validate_exp = true;
	let token_data = decode::<AdminClaims>(token, &DecodingKey::from_secret(secret.as_bytes()), &validation)
		.map_err(|_| AppError::Unauthorized)?;
	if token_data.claims.role != "admin" {
		return Err(AppError::Unauthorized);
	}
	Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Keypair, Signer as _};

    #[test]
    fn test_verify_wallet_signature_roundtrip() {
        let mut csprng = rand::rngs::OsRng;
        let kp: Keypair = Keypair::generate(&mut csprng);
        let message = b"hello-vault";
        let sig = kp.sign(message);
        let owner = bs58::encode(kp.public.as_bytes()).into_string();
        let signature = bs58::encode(sig.to_bytes()).into_string();
        assert!(verify_wallet_signature(&owner, message, &signature).is_ok());
    }
}


