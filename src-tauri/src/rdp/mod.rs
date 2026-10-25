pub(crate) mod clipboard;
pub(crate) mod connect;
pub(crate) mod input;
pub(crate) mod session;
#[cfg(test)]
pub(crate) mod testserver;

use crate::error::AppError;

fn err(msg: impl Into<String>) -> AppError {
    AppError::Internal(format!("rdp: {}", msg.into()))
}
