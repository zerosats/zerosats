use actix_web::{HttpResponse, ResponseError, http::StatusCode};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("internal: {0}")]
    Internal(String),
    #[error("Invalid payment hash format")]
    InvalidPaymentHash,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::InvalidPaymentHash => StatusCode::BAD_REQUEST,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        let kind = match self {
            Self::InvalidPaymentHash => "bad_request",
            Self::BadRequest(_) => "bad_request",
            Self::NotFound => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Unavailable(_) => "unavailable",
            Self::Internal(_) => "internal",
        };
        HttpResponse::build(self.status_code()).json(ErrorBody {
            error: kind.into(),
            message: self.to_string(),
        })
    }
}

impl From<crate::db::quotes::Error> for ApiError {
    fn from(e: crate::db::quotes::Error) -> Self {
        use crate::db::quotes::Error as DbErr;
        match e {
            DbErr::NotFound => ApiError::NotFound,
            DbErr::DuplicatePaymentHash => {
                ApiError::Conflict("payment_hash already in use".into())
            }
            DbErr::DuplicateNoteCommitment => {
                ApiError::Conflict("note_commitment already in use".into())
            }
            DbErr::Sqlx(e) => ApiError::Internal(format!("db error: {e}")),
            DbErr::Row(e) => ApiError::Internal(format!("row decode: {e}")),
        }
    }
}

impl From<crate::db::onramps::Error> for ApiError {
    fn from(e: crate::db::onramps::Error) -> Self {
        use crate::db::onramps::Error as DbErr;
        match e {
            DbErr::NotFound => ApiError::NotFound,
            DbErr::DuplicatePaymentHash => {
                ApiError::Conflict("payment_hash already in use".into())
            }
            DbErr::Sqlx(e) => ApiError::Internal(format!("db error: {e}")),
            DbErr::Row(e) => ApiError::Internal(format!("row decode: {e}")),
        }
    }
}
