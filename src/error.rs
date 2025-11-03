use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
	#[error("Database error: {0}")]
	Db(#[from] sqlx::Error),
	#[error("Solana error: {0}")]
	Solana(String),
	#[error("Bad request: {0}")]
	BadRequest(String),
	#[error("Unauthorized")]
	Unauthorized,
	#[error("Internal error: {0}")]
	Internal(String),
}

#[derive(Serialize)]
struct ErrorBody {
	code: u16,
	message: String,
}

impl IntoResponse for AppError {
	fn into_response(self) -> axum::response::Response {
		let (status, message) = match self {
			AppError::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
			AppError::Solana(e) => (StatusCode::BAD_GATEWAY, e),
			AppError::BadRequest(e) => (StatusCode::BAD_REQUEST, e),
			AppError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized".to_string()),
			AppError::Internal(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
		};
		let body = Json(ErrorBody { code: status.as_u16(), message });
		(status, body).into_response()
	}
}

pub type AppResult<T> = Result<T, AppError>;


