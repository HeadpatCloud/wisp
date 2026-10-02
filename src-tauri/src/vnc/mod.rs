pub mod ard;
pub mod decode;
pub mod handshake;
pub mod proto;
pub mod session;
#[cfg(test)]
pub(crate) mod testserver;
pub mod vencrypt;

use crate::error::{AppError, AppResult};

fn err(msg: impl Into<String>) -> AppError {
    AppError::Internal(format!("vnc: {}", msg.into()))
}

// Caps on server-declared lengths so a hostile server can't trigger a huge allocation.
const MAX_TEXT: usize = 1 << 20; // failure reason, desktop name, clipboard

fn bounded(len: usize, max: usize) -> AppResult<usize> {
    if len > max {
        return Err(err(format!("declared length {len} exceeds {max}")));
    }
    Ok(len)
}

// SetPixelFormat body (16 bytes after the message-type + 3 padding bytes that the caller
// prepends) requesting a fixed 32bpp little-endian true-colour format (red_shift=16, green=8,
// blue=0), so a raw pixel's little-endian bytes are [blue, green, red, x].
pub const PIXEL_FORMAT: [u8; 16] =
    [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];
