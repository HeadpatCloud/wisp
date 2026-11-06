pub(crate) mod clipboard;
pub(crate) mod connect;
pub(crate) mod drive;
pub(crate) mod gateway;
#[cfg(test)]
pub(crate) mod gatewaytest;
pub(crate) mod input;
pub(crate) mod session;
pub(crate) mod share;
pub(crate) mod sound;
#[cfg(test)]
pub(crate) mod testserver;

use crate::error::AppError;

fn err(msg: impl Into<String>) -> AppError {
    AppError::Internal(format!("rdp: {}", msg.into()))
}
