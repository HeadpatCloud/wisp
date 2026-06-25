use des::cipher::{Block, BlockCipherEncrypt, KeyInit};
use des::Des;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use crate::error::{AppError, AppResult};

fn err(msg: impl Into<String>) -> AppError {
    AppError::Internal(format!("vnc: {}", msg.into()))
}

// Caps on server-declared lengths so a hostile server can't trigger a huge allocation.
const MAX_TEXT: usize = 1 << 20; // failure reason, desktop name, clipboard
const MAX_RECT: usize = 256 << 20; // one raw framebuffer rectangle

fn bounded(len: usize, max: usize) -> AppResult<usize> {
    if len > max {
        return Err(err(format!("declared length {len} exceeds {max}")));
    }
    Ok(len)
}

pub struct VncInit {
    pub reader: OwnedReadHalf,
    pub writer: OwnedWriteHalf,
    pub width: u16,
    pub height: u16,
}

// RFB 3.8 client handshake: version, security (None or VNC-password), ServerInit,
// then request our fixed 32bpp format and Raw-only encoding.
pub async fn connect(host: &str, port: u16, password: &str) -> AppResult<VncInit> {
    let mut stream = TcpStream::connect((host, port)).await?;

    let mut version = [0u8; 12];
    stream.read_exact(&mut version).await?;
    stream.write_all(b"RFB 003.008\n").await?;

    let mut count = [0u8; 1];
    stream.read_exact(&mut count).await?;
    if count[0] == 0 {
        let mut len = [0u8; 4];
        stream.read_exact(&mut len).await?;
        let mut reason = vec![0u8; bounded(u32::from_be_bytes(len) as usize, MAX_TEXT)?];
        stream.read_exact(&mut reason).await?;
        return Err(err(String::from_utf8_lossy(&reason).into_owned()));
    }
    let mut types = vec![0u8; count[0] as usize];
    stream.read_exact(&mut types).await?;
    // A password-protected session must never silently fall back to "None" auth: a hostile
    // server could advertise type 1 to skip the password and connect us unauthenticated.
    let security = if !password.is_empty() && types.contains(&2) {
        2u8
    } else if types.contains(&1) {
        1u8
    } else if types.contains(&2) {
        2u8
    } else {
        return Err(err("no supported security type"));
    };
    stream.write_all(&[security]).await?;

    if security == 2 {
        let mut challenge = [0u8; 16];
        stream.read_exact(&mut challenge).await?;
        stream.write_all(&vnc_auth_response(password, &challenge)).await?;
    }

    let mut result = [0u8; 4];
    stream.read_exact(&mut result).await?;
    if u32::from_be_bytes(result) != 0 {
        return Err(err("authentication failed"));
    }

    stream.write_all(&[1]).await?; // ClientInit: shared

    let mut header = [0u8; 24]; // width, height, pixel-format(16), name-length
    stream.read_exact(&mut header).await?;
    let width = u16::from_be_bytes([header[0], header[1]]);
    let height = u16::from_be_bytes([header[2], header[3]]);
    let name_len = u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
    let mut name = vec![0u8; bounded(name_len, MAX_TEXT)?];
    stream.read_exact(&mut name).await?;

    let mut set_format = [0u8; 20];
    set_format[4..].copy_from_slice(&PIXEL_FORMAT);
    stream.write_all(&set_format).await?;
    // SetEncodings: Hextile (5), CopyRect (1), Raw (0) - server picks the best it has.
    stream.write_all(&[2, 0, 0, 3, 0, 0, 0, 5, 0, 0, 0, 1, 0, 0, 0, 0]).await?;

    let (reader, writer) = stream.into_split();
    Ok(VncInit { reader, writer, width, height })
}

// One decoded screen operation to apply to the canvas.
pub enum DrawOp {
    Raw { x: u16, y: u16, w: u16, h: u16, rgba: Vec<u8> },
    Copy { x: u16, y: u16, w: u16, h: u16, src_x: u16, src_y: u16 },
}

// Write one source pixel (BGRX in our negotiated format) to the RGBA buffer.
fn put_px(rgba: &mut [u8], stride: usize, x: usize, y: usize, px: &[u8; 4]) {
    let i = (y * stride + x) * 4;
    rgba[i] = px[2];
    rgba[i + 1] = px[1];
    rgba[i + 2] = px[0];
    rgba[i + 3] = 255;
}

// Decode a Hextile rectangle (16x16 tiles, background/foreground carried across
// tiles, optional sub-rectangles) into an RGBA buffer.
async fn decode_hextile<R: AsyncRead + Unpin>(reader: &mut R, w: u16, h: u16) -> AppResult<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    let mut rgba = vec![0u8; bounded(w * h * 4, MAX_RECT)?];
    let mut bg = [0u8; 4];
    let mut fg = [0u8; 4];
    let mut ty = 0;
    while ty < h {
        let th = (h - ty).min(16);
        let mut tx = 0;
        while tx < w {
            let tw = (w - tx).min(16);
            let mut sub = [0u8; 1];
            reader.read_exact(&mut sub).await?;
            let mask = sub[0];
            if mask & 0x01 != 0 {
                let mut raw = vec![0u8; tw * th * 4];
                reader.read_exact(&mut raw).await?;
                for row in 0..th {
                    for col in 0..tw {
                        let p = &raw[(row * tw + col) * 4..];
                        put_px(&mut rgba, w, tx + col, ty + row, &[p[0], p[1], p[2], p[3]]);
                    }
                }
            } else {
                if mask & 0x02 != 0 {
                    reader.read_exact(&mut bg).await?;
                }
                if mask & 0x04 != 0 {
                    reader.read_exact(&mut fg).await?;
                }
                for row in 0..th {
                    for col in 0..tw {
                        put_px(&mut rgba, w, tx + col, ty + row, &bg);
                    }
                }
                if mask & 0x08 != 0 {
                    let mut n = [0u8; 1];
                    reader.read_exact(&mut n).await?;
                    for _ in 0..n[0] {
                        let mut color = fg;
                        if mask & 0x10 != 0 {
                            reader.read_exact(&mut color).await?;
                        }
                        let mut xy = [0u8; 2];
                        reader.read_exact(&mut xy).await?;
                        let (sx, sy) = ((xy[0] >> 4) as usize, (xy[0] & 0x0f) as usize);
                        let (sw, sh) = (((xy[1] >> 4) + 1) as usize, ((xy[1] & 0x0f) + 1) as usize);
                        for row in 0..sh {
                            for col in 0..sw {
                                put_px(&mut rgba, w, tx + sx + col, ty + sy + row, &color);
                            }
                        }
                    }
                }
            }
            tx += 16;
        }
        ty += 16;
    }
    Ok(rgba)
}

pub enum ServerMsg {
    Frame(Vec<DrawOp>),
    Clipboard(String),
    Ignored,
}

// Read one server message: a framebuffer update, a clipboard cut-text, or an
// ignored message. Errors on a closed stream.
pub async fn read_message(reader: &mut OwnedReadHalf) -> AppResult<ServerMsg> {
    let mut kind = [0u8; 1];
    reader.read_exact(&mut kind).await?;
    match kind[0] {
        0 => {
            let mut head = [0u8; 3]; // padding(1) + rect-count(2)
            reader.read_exact(&mut head).await?;
            let rects = u16::from_be_bytes([head[1], head[2]]);
            let mut out = Vec::with_capacity(rects as usize);
            for _ in 0..rects {
                let mut r = [0u8; 12]; // x,y,w,h (u16) + encoding (i32)
                reader.read_exact(&mut r).await?;
                let x = u16::from_be_bytes([r[0], r[1]]);
                let y = u16::from_be_bytes([r[2], r[3]]);
                let w = u16::from_be_bytes([r[4], r[5]]);
                let h = u16::from_be_bytes([r[6], r[7]]);
                let encoding = i32::from_be_bytes([r[8], r[9], r[10], r[11]]);
                match encoding {
                    0 => {
                        let mut pixels = vec![0u8; bounded(w as usize * h as usize * 4, MAX_RECT)?];
                        reader.read_exact(&mut pixels).await?;
                        out.push(DrawOp::Raw { x, y, w, h, rgba: raw_to_rgba(&pixels) });
                    }
                    1 => {
                        let mut s = [0u8; 4];
                        reader.read_exact(&mut s).await?;
                        let src_x = u16::from_be_bytes([s[0], s[1]]);
                        let src_y = u16::from_be_bytes([s[2], s[3]]);
                        out.push(DrawOp::Copy { x, y, w, h, src_x, src_y });
                    }
                    5 => {
                        let rgba = decode_hextile(reader, w, h).await?;
                        out.push(DrawOp::Raw { x, y, w, h, rgba });
                    }
                    other => return Err(err(format!("unsupported encoding {other}"))),
                }
            }
            Ok(ServerMsg::Frame(out))
        }
        2 => Ok(ServerMsg::Ignored), // Bell
        3 => {
            let mut head = [0u8; 7]; // padding(3) + length(4)
            reader.read_exact(&mut head).await?;
            let len = bounded(u32::from_be_bytes([head[3], head[4], head[5], head[6]]) as usize, MAX_TEXT)?;
            let mut text = vec![0u8; len];
            reader.read_exact(&mut text).await?;
            Ok(ServerMsg::Clipboard(String::from_utf8_lossy(&text).into_owned()))
        }
        other => Err(err(format!("unexpected server message {other}"))),
    }
}

// VNC authentication (RFB security type 2): each password byte has its bits
// reversed (a VNC quirk), the first 8 bytes form a DES key, and the 16-byte
// challenge is ECB-encrypted as two 8-byte blocks.
pub fn vnc_auth_response(password: &str, challenge: &[u8; 16]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (slot, b) in key.iter_mut().zip(password.bytes()) {
        *slot = b.reverse_bits();
    }
    let cipher = Des::new_from_slice(&key).expect("8-byte DES key");
    let mut out = *challenge;
    for block in out.chunks_mut(8) {
        let mut b = Block::<Des>::try_from(&*block).expect("8-byte block");
        cipher.encrypt_block(&mut b);
        block.copy_from_slice(&b);
    }
    out
}

// We always negotiate a fixed 32bpp little-endian true-colour format
// (red_shift=16, green=8, blue=0), so a raw pixel's little-endian bytes are
// [blue, green, red, x]. Convert to canvas RGBA.
pub fn raw_to_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len());
    for px in pixels.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    out
}

// SetPixelFormat body requesting the fixed format above (16 bytes after the
// message-type + 3 padding bytes that the caller prepends).
pub const PIXEL_FORMAT: [u8; 16] =
    [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];

pub fn fb_update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let mut b = [0u8; 10];
    b[0] = 3;
    b[1] = u8::from(incremental);
    b[2..4].copy_from_slice(&x.to_be_bytes());
    b[4..6].copy_from_slice(&y.to_be_bytes());
    b[6..8].copy_from_slice(&w.to_be_bytes());
    b[8..10].copy_from_slice(&h.to_be_bytes());
    b
}

pub fn pointer_event(button_mask: u8, x: u16, y: u16) -> [u8; 6] {
    let mut b = [0u8; 6];
    b[0] = 5;
    b[1] = button_mask;
    b[2..4].copy_from_slice(&x.to_be_bytes());
    b[4..6].copy_from_slice(&y.to_be_bytes());
    b
}

pub fn key_event(down: bool, keysym: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0] = 4;
    b[1] = u8::from(down);
    b[4..8].copy_from_slice(&keysym.to_be_bytes());
    b
}

pub fn client_cut_text(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut msg = vec![6, 0, 0, 0]; // type + 3 padding
    msg.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    msg.extend_from_slice(bytes);
    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_response_is_deterministic_and_password_dependent() {
        let challenge = [7u8; 16];
        let a = vnc_auth_response("hunter2", &challenge);
        let b = vnc_auth_response("hunter2", &challenge);
        let c = vnc_auth_response("other", &challenge);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn raw_pixels_convert_bgrx_to_rgba() {
        // one pixel: blue=10, green=20, red=30, pad=0
        let rgba = raw_to_rgba(&[10, 20, 30, 0]);
        assert_eq!(rgba, [30, 20, 10, 255]);
    }

    #[tokio::test]
    async fn hextile_background_with_foreground_subrect() {
        // One 2x2 tile: bg+fg specified, one 1x1 sub-rect at (1,0) using fg.
        let mut data = vec![0x02 | 0x04 | 0x08]; // bg + fg + any-subrects
        data.extend_from_slice(&[255, 0, 0, 0]); // bg = blue (B,G,R,X)
        data.extend_from_slice(&[0, 0, 255, 0]); // fg = red
        data.push(1); // one sub-rect
        data.extend_from_slice(&[0x10, 0x00]); // x=1 y=0, w=1 h=1
        let mut r: &[u8] = &data;
        let rgba = decode_hextile(&mut r, 2, 2).await.unwrap();
        assert_eq!(&rgba[0..4], &[0, 0, 255, 255]); // (0,0) blue
        assert_eq!(&rgba[4..8], &[255, 0, 0, 255]); // (1,0) red sub-rect
        assert_eq!(&rgba[8..12], &[0, 0, 255, 255]); // (0,1) blue
    }

    #[test]
    fn input_events_have_correct_layout() {
        assert_eq!(pointer_event(0b10, 0x0102, 0x0304), [5, 2, 1, 2, 3, 4]);
        assert_eq!(key_event(true, 0x0041), [4, 1, 0, 0, 0, 0, 0, 0x41]);
        assert_eq!(fb_update_request(true, 0, 0, 0x0102, 0x0304), [3, 1, 0, 0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(client_cut_text("hi"), [6, 0, 0, 0, 0, 0, 0, 2, b'h', b'i']);
    }
}
