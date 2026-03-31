// SPDX-License-Identifier: BSL-1.1

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::types::ErrorBody;

pub fn internal_err(msg: &str) -> (StatusCode, Json<ErrorBody>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "Internal".into(),
            message: msg.into(),
        }),
    )
}

/// HTTP Result type
#[allow(dead_code)]
pub type HttpResult<T> = Result<T, HttpError>;

/// HTTP Error wrapper
#[allow(dead_code)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
}

#[allow(dead_code)]
impl HttpError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self
                    .status
                    .canonical_reason()
                    .unwrap_or("Error")
                    .to_string(),
                message: self.message,
            }),
        )
            .into_response()
    }
}
