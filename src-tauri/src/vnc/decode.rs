use flate2::{Decompress, FlushDecompress};
use tokio::io::{AsyncRead, AsyncReadExt};

use super::err;
use crate::error::{AppError, AppResult};
use crate::remote::FrameOp;

pub const ENCODINGS: [i32; 7] = [16, 5, 1, 0, -239, -223, -308];

const MAX_ZRLE: usize = 64 << 20;
const MAX_CURSOR: usize = 1024;
// An 8K screen has 33.2 million pixels.
const MAX_DESKTOP: usize = 40_000_000;

fn invalid() -> AppError {
    err("the server sent an invalid update")
}

// A pixel as the server sends it (blue, green, red) to RGBA.
fn rgba(px: &[u8]) -> [u8; 4] {
    [px[2], px[1], px[0], 255]
}

fn fill(out: &mut [u8], stride: usize, x: usize, y: usize, w: usize, h: usize, colour: [u8; 4]) {
    for row in y..y + h {
        let at = (row * stride + x) * 4;
        for px in out[at..at + w * 4].chunks_exact_mut(4) {
            px.copy_from_slice(&colour);
        }
    }
}

async fn pixel<R: AsyncRead + Unpin>(r: &mut R) -> AppResult<[u8; 4]> {
    let mut px = [0u8; 4];
    r.read_exact(&mut px).await?;
    Ok(rgba(&px))
}

async fn hextile<R: AsyncRead + Unpin>(
    r: &mut R,
    out: &mut [u8],
    w: usize,
    h: usize,
) -> AppResult<()> {
    let (mut bg, mut fg) = ([0, 0, 0, 255], [0, 0, 0, 255]);
    let mut buf = [0u8; 255 * 6];
    for ty in (0..h).step_by(16) {
        let th = (h - ty).min(16);
        for tx in (0..w).step_by(16) {
            let tw = (w - tx).min(16);
            let mask = r.read_u8().await?;
            if mask & 1 != 0 {
                let raw = &mut buf[..tw * th * 4];
                r.read_exact(raw).await?;
                for (row, line) in raw.chunks_exact(tw * 4).enumerate() {
                    let at = ((ty + row) * w + tx) * 4;
                    let drawn = out[at..at + tw * 4].chunks_exact_mut(4);
                    for (px, sent) in drawn.zip(line.chunks_exact(4)) {
                        px.copy_from_slice(&rgba(sent));
                    }
                }
                continue;
            }
            if mask & 2 != 0 {
                bg = pixel(r).await?;
            }
            if mask & 4 != 0 {
                fg = pixel(r).await?;
            }
            fill(out, w, tx, ty, tw, th, bg);
            if mask & 8 == 0 {
                continue;
            }
            let count = r.read_u8().await? as usize;
            let size = if mask & 16 != 0 { 6 } else { 2 };
            let subrects = &mut buf[..count * size];
            r.read_exact(subrects).await?;
            for sub in subrects.chunks_exact(size) {
                let (colour, place) = if size == 6 { (rgba(sub), &sub[4..]) } else { (fg, sub) };
                let (sx, sy) = ((place[0] >> 4) as usize, (place[0] & 15) as usize);
                let (sw, sh) = ((place[1] >> 4) as usize + 1, (place[1] & 15) as usize + 1);
                if sx + sw > tw || sy + sh > th {
                    return Err(invalid());
                }
                fill(out, w, tx + sx, ty + sy, sw, sh, colour);
            }
        }
    }
    Ok(())
}

// The zlib data of one ZRLE rectangle, inflated only as far as its tiles ask for.
struct Inflater<'a> {
    zlib: &'a mut Decompress,
    input: &'a [u8],
}

impl Inflater<'_> {
    // One call into zlib: the bytes it took from the input and the bytes it put into `out`.
    fn step(&mut self, out: &mut [u8]) -> AppResult<(usize, usize)> {
        let (taken, given) = (self.zlib.total_in(), self.zlib.total_out());
        self.zlib.decompress(self.input, out, FlushDecompress::None).map_err(|_| invalid())?;
        let taken = (self.zlib.total_in() - taken) as usize;
        self.input = &self.input[taken..];
        Ok((taken, (self.zlib.total_out() - given) as usize))
    }

    fn read(&mut self, out: &mut [u8]) -> AppResult<()> {
        let mut filled = 0;
        while filled < out.len() {
            let (taken, given) = self.step(&mut out[filled..])?;
            if taken == 0 && given == 0 {
                return Err(invalid());
            }
            filled += given;
        }
        Ok(())
    }

    fn byte(&mut self) -> AppResult<u8> {
        let mut b = [0u8; 1];
        self.read(&mut b)?;
        Ok(b[0])
    }

    fn cpixel(&mut self) -> AppResult<[u8; 4]> {
        let mut px = [0u8; 3];
        self.read(&mut px)?;
        Ok(rgba(&px))
    }

    fn cpixels(&mut self, out: &mut [[u8; 4]]) -> AppResult<()> {
        let mut raw = [0u8; 64 * 64 * 3];
        let raw = &mut raw[..out.len() * 3];
        self.read(raw)?;
        for (px, sent) in out.iter_mut().zip(raw.chunks_exact(3)) {
            *px = rgba(sent);
        }
        Ok(())
    }

    // 1 + the length bytes up to the first that is not 255; `left` is what the tile still takes.
    fn run(&mut self, left: usize) -> AppResult<usize> {
        let mut len = 1;
        loop {
            let b = self.byte()?;
            len += b as usize;
            if len > left {
                return Err(invalid());
            }
            if b != 255 {
                return Ok(len);
            }
        }
    }

    fn tile(&mut self, tile: &mut [[u8; 4]], tw: usize) -> AppResult<()> {
        let mut palette = [[0u8; 4]; 127];
        match self.byte()? {
            0 => self.cpixels(tile)?,
            1 => tile.fill(self.cpixel()?),
            sub @ 2..=16 => {
                let palette = &mut palette[..sub as usize];
                self.cpixels(palette)?;
                let bits = match sub {
                    2 => 1,
                    3..=4 => 2,
                    _ => 4,
                };
                let mut packed = [0u8; 32];
                let packed = &mut packed[..(tw * bits).div_ceil(8)];
                for row in tile.chunks_exact_mut(tw) {
                    self.read(packed)?;
                    for (col, px) in row.iter_mut().enumerate() {
                        let shift = 8 - bits - (col * bits) % 8;
                        let index = (packed[col * bits / 8] >> shift) & ((1 << bits) - 1);
                        *px = *palette.get(index as usize).ok_or_else(invalid)?;
                    }
                }
            }
            128 => {
                let mut done = 0;
                while done < tile.len() {
                    let colour = self.cpixel()?;
                    let run = self.run(tile.len() - done)?;
                    tile[done..done + run].fill(colour);
                    done += run;
                }
            }
            sub @ 130..=255 => {
                let palette = &mut palette[..sub as usize - 128];
                self.cpixels(palette)?;
                let mut done = 0;
                while done < tile.len() {
                    let index = self.byte()?;
                    let colour = *palette.get((index & 127) as usize).ok_or_else(invalid)?;
                    let run = if index & 128 != 0 { self.run(tile.len() - done)? } else { 1 };
                    tile[done..done + run].fill(colour);
                    done += run;
                }
            }
            _ => return Err(invalid()),
        }
        Ok(())
    }

    // The rest of the rectangle's data has to go through zlib too, and may not hold more pixels.
    fn finish(mut self) -> AppResult<()> {
        loop {
            let (taken, given) = self.step(&mut [0u8; 1])?;
            if given != 0 {
                return Err(invalid());
            }
            if taken == 0 {
                break;
            }
        }
        if !self.input.is_empty() {
            return Err(invalid());
        }
        Ok(())
    }
}

fn zrle(zlib: &mut Decompress, data: &[u8], out: &mut [u8], w: usize, h: usize) -> AppResult<()> {
    let mut inflater = Inflater { zlib, input: data };
    let mut tile = [[0u8; 4]; 64 * 64];
    for ty in (0..h).step_by(64) {
        let th = (h - ty).min(64);
        for tx in (0..w).step_by(64) {
            let tw = (w - tx).min(64);
            let tile = &mut tile[..tw * th];
            inflater.tile(tile, tw)?;
            for (row, line) in tile.chunks_exact(tw).enumerate() {
                let at = ((ty + row) * w + tx) * 4;
                out[at..at + tw * 4].copy_from_slice(line.as_flattened());
            }
        }
    }
    inflater.finish()
}

async fn cursor<R: AsyncRead + Unpin>(
    r: &mut R,
    hot_x: u16,
    hot_y: u16,
    w: u16,
    h: u16,
) -> AppResult<FrameOp> {
    let (width, height) = (w as usize, h as usize);
    if width > MAX_CURSOR || height > MAX_CURSOR {
        return Err(invalid());
    }
    if width == 0 || height == 0 {
        return Ok(FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() });
    }
    let mut rgba = vec![0u8; width * height * 4];
    r.read_exact(&mut rgba).await?;
    let row_bytes = width.div_ceil(8);
    let mut mask = vec![0u8; row_bytes * height];
    r.read_exact(&mut mask).await?;
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let (row, col) = (i / width, i % width);
        let opaque = mask[row * row_bytes + col / 8] & (0x80 >> (col % 8)) != 0;
        px.swap(0, 2);
        px[3] = if opaque { 255 } else { 0 };
    }
    // TightVNC sends a 1x2 cursor with its hotspot at 0,2.
    Ok(FrameOp::Cursor { hot_x: hot_x.min(w - 1), hot_y: hot_y.min(h - 1), w, h, rgba })
}

fn desktop_size(w: u16, h: u16) -> AppResult<()> {
    if w == 0 || h == 0 || w as usize * h as usize > MAX_DESKTOP {
        return Err(err("the server reported an invalid desktop size"));
    }
    Ok(())
}

pub struct Decoder {
    width: u16,
    height: u16,
    zlib: Decompress,
}

impl Decoder {
    pub fn new(width: u16, height: u16) -> AppResult<Self> {
        desktop_size(width, height)?;
        Ok(Self { width, height, zlib: Decompress::new(true) })
    }

    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    fn on_screen(&self, x: u16, y: u16, w: u16, h: u16) -> AppResult<()> {
        let fits = x as u32 + w as u32 <= self.width as u32
            && y as u32 + h as u32 <= self.height as u32;
        if !fits {
            return Err(invalid());
        }
        Ok(())
    }

    fn count(&self, drawn: &mut usize, w: u16, h: u16) -> AppResult<()> {
        *drawn += w as usize * h as usize;
        if *drawn > 4 * self.width as usize * self.height as usize {
            return Err(invalid());
        }
        Ok(())
    }

    // Servers announce the size they already have in answer to a full request (TigerVNC, x11vnc
    // and QEMU do, without any pixels): that is not a resize.
    fn resize(&mut self, w: u16, h: u16) -> AppResult<Option<FrameOp>> {
        desktop_size(w, h)?;
        if (w, h) == (self.width, self.height) {
            return Ok(None);
        }
        (self.width, self.height) = (w, h);
        Ok(Some(FrameOp::Resize { w, h }))
    }

    // Reads one FramebufferUpdate body (after the message-type byte) and hands each operation to
    // the sink as soon as it is decoded.
    pub async fn update<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
        sink: &mut impl FnMut(FrameOp),
    ) -> AppResult<()> {
        let mut head = [0u8; 3];
        r.read_exact(&mut head).await?;
        // Pixels drawn since the update began or the screen was resized. Rectangles cost a server
        // next to nothing, so an update may cover the screen four times and no more.
        let mut drawn = 0;
        for _ in 0..u16::from_be_bytes([head[1], head[2]]) {
            let mut rect = [0u8; 12];
            r.read_exact(&mut rect).await?;
            let [x, y, w, h] = [0, 2, 4, 6].map(|at| u16::from_be_bytes([rect[at], rect[at + 1]]));
            let encoding = i32::from_be_bytes([rect[8], rect[9], rect[10], rect[11]]);
            match encoding {
                0 | 5 | 16 => {
                    self.on_screen(x, y, w, h)?;
                    self.count(&mut drawn, w, h)?;
                    let (width, height) = (w as usize, h as usize);
                    let mut rgba = vec![0u8; width * height * 4];
                    match encoding {
                        0 => {
                            r.read_exact(&mut rgba).await?;
                            for px in rgba.chunks_exact_mut(4) {
                                px.swap(0, 2);
                                px[3] = 255;
                            }
                        }
                        5 => hextile(r, &mut rgba, width, height).await?,
                        _ => {
                            let compressed = r.read_u32().await? as usize;
                            if compressed > MAX_ZRLE {
                                return Err(invalid());
                            }
                            let mut data = vec![0u8; compressed];
                            r.read_exact(&mut data).await?;
                            zrle(&mut self.zlib, &data, &mut rgba, width, height)?;
                        }
                    }
                    // Nothing to draw, and a canvas refuses an image without pixels.
                    if !rgba.is_empty() {
                        sink(FrameOp::Rect { x, y, w, h, rgba });
                    }
                }
                1 => {
                    let (src_x, src_y) = (r.read_u16().await?, r.read_u16().await?);
                    self.on_screen(x, y, w, h)?;
                    self.on_screen(src_x, src_y, w, h)?;
                    self.count(&mut drawn, w, h)?;
                    if w != 0 && h != 0 {
                        sink(FrameOp::Copy { x, y, w, h, src_x, src_y });
                    }
                }
                -223 => {
                    if let Some(op) = self.resize(w, h)? {
                        drawn = 0;
                        sink(op);
                    }
                }
                -308 => {
                    let screens = r.read_u8().await? as usize;
                    let mut skipped = [0u8; 3 + 255 * 16];
                    r.read_exact(&mut skipped[..3 + screens * 16]).await?;
                    // x is the reason. Only for 1, the reply to a request of this client, is y a
                    // status, and not 0 then means the server refused.
                    if x != 1 || y == 0 {
                        if let Some(op) = self.resize(w, h)? {
                            drawn = 0;
                            sink(op);
                        }
                    }
                }
                -239 => sink(cursor(r, x, y, w, h).await?),
                other => return Err(err(format!("unsupported encoding {other}"))),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flate2::{Compress, Compression, FlushCompress};

    use super::*;
    use crate::error::AppError;

    fn rect(x: u16, y: u16, w: u16, h: u16, encoding: i32, data: &[u8]) -> Vec<u8> {
        let mut b = Vec::new();
        for field in [x, y, w, h] {
            b.extend(field.to_be_bytes());
        }
        b.extend(encoding.to_be_bytes());
        b.extend(data);
        b
    }

    fn update(rects: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0];
        b.extend((rects.len() as u16).to_be_bytes());
        b.extend(rects.concat());
        b
    }

    // Three distinct colours per `n`: as the server sends them, and as they are drawn.
    fn cpixel(n: u8) -> [u8; 3] {
        [n, n + 1, n + 2]
    }

    fn sent(n: u8) -> [u8; 4] {
        [n, n + 1, n + 2, 0xEE]
    }

    fn shown(n: u8) -> [u8; 4] {
        [n + 2, n + 1, n, 255]
    }

    fn picture(pixels: impl IntoIterator<Item = u8>) -> Vec<u8> {
        pixels.into_iter().flat_map(shown).collect()
    }

    fn sized(w: u16, h: u16) -> Decoder {
        Decoder::new(w, h).unwrap()
    }

    // What the sink was handed, and how the update ended.
    async fn decode_some(decoder: &mut Decoder, bytes: &[u8]) -> (Vec<FrameOp>, AppResult<()>) {
        let mut reader = bytes;
        let mut ops = Vec::new();
        let result = decoder.update(&mut reader, &mut |op| ops.push(op)).await;
        if result.is_ok() {
            assert!(reader.is_empty(), "{} bytes left unread", reader.len());
        }
        for op in &ops {
            if let FrameOp::Rect { w, h, rgba, .. } | FrameOp::Cursor { w, h, rgba, .. } = op {
                assert_eq!(rgba.len(), *w as usize * *h as usize * 4);
            }
        }
        (ops, result)
    }

    async fn decode(decoder: &mut Decoder, bytes: &[u8]) -> AppResult<Vec<FrameOp>> {
        let (ops, result) = decode_some(decoder, bytes).await;
        result.map(|()| ops)
    }

    async fn assert_invalid(decoder: &mut Decoder, bytes: &[u8]) {
        match decode(decoder, bytes).await {
            Err(AppError::Internal(message)) => {
                assert_eq!(message, "vnc: the server sent an invalid update")
            }
            other => panic!("expected an invalid update, got {other:?}"),
        }
    }

    fn compressor() -> Compress {
        Compress::new(Compression::default(), true)
    }

    // One rectangle's tile data, ended on a sync flush as a server does.
    fn deflate(zlib: &mut Compress, tiles: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(tiles.len() * 2 + 64);
        let before = zlib.total_in();
        zlib.compress_vec(tiles, &mut out, FlushCompress::Sync).unwrap();
        assert_eq!(zlib.total_in() - before, tiles.len() as u64);
        out
    }

    fn zrle_data(x: u16, y: u16, w: u16, h: u16, data: &[u8]) -> Vec<u8> {
        let mut body = (data.len() as u32).to_be_bytes().to_vec();
        body.extend(data);
        rect(x, y, w, h, 16, &body)
    }

    fn zrle_rect(zlib: &mut Compress, x: u16, y: u16, w: u16, h: u16, tiles: &[u8]) -> Vec<u8> {
        zrle_data(x, y, w, h, &deflate(zlib, tiles))
    }

    // What a new connection makes of one ZRLE rectangle that covers a w x h screen.
    async fn zrle_screen(w: u16, h: u16, tiles: &[u8]) -> AppResult<Vec<FrameOp>> {
        let bytes = update(&[zrle_rect(&mut compressor(), 0, 0, w, h, tiles)]);
        decode(&mut sized(w, h), &bytes).await
    }

    async fn assert_zrle_invalid(w: u16, h: u16, tiles: &[u8]) {
        let bytes = update(&[zrle_rect(&mut compressor(), 0, 0, w, h, tiles)]);
        assert_invalid(&mut sized(w, h), &bytes).await;
    }

    fn screen(w: u16, h: u16, pixels: impl IntoIterator<Item = u8>) -> Vec<FrameOp> {
        vec![FrameOp::Rect { x: 0, y: 0, w, h, rgba: picture(pixels) }]
    }

    #[tokio::test]
    async fn an_update_without_rectangles_has_no_operations() {
        let ops = decode(&mut sized(4, 4), &update(&[])).await.unwrap();
        assert!(ops.is_empty());
    }

    #[tokio::test]
    async fn raw_pixels_become_rgba() {
        let data = [10, 20, 30, 99, 40, 50, 60, 0];
        let ops = decode(&mut sized(4, 4), &update(&[rect(1, 2, 2, 1, 0, &data)])).await;
        let rgba = vec![30, 20, 10, 255, 60, 50, 40, 255];
        assert_eq!(ops.unwrap(), [FrameOp::Rect { x: 1, y: 2, w: 2, h: 1, rgba }]);
    }

    #[tokio::test]
    async fn copy_rect_becomes_a_copy() {
        let bytes = update(&[rect(5, 6, 3, 2, 1, &[0, 1, 0, 2])]);
        let ops = decode(&mut sized(10, 10), &bytes).await.unwrap();
        assert_eq!(ops, [FrameOp::Copy { x: 5, y: 6, w: 3, h: 2, src_x: 1, src_y: 2 }]);
    }

    #[tokio::test]
    async fn rectangles_come_out_in_the_order_they_were_sent() {
        let bytes = update(&[
            rect(0, 0, 1, 1, 0, &sent(10)),
            rect(1, 0, 1, 1, 1, &[0, 0, 0, 0]),
            rect(2, 0, 1, 1, 5, &[1, 20, 21, 22, 0]),
        ]);
        let ops = decode(&mut sized(4, 4), &bytes).await.unwrap();
        assert_eq!(
            ops,
            [
                FrameOp::Rect { x: 0, y: 0, w: 1, h: 1, rgba: picture([10]) },
                FrameOp::Copy { x: 1, y: 0, w: 1, h: 1, src_x: 0, src_y: 0 },
                FrameOp::Rect { x: 2, y: 0, w: 1, h: 1, rgba: picture([20]) },
            ],
        );
    }

    #[tokio::test]
    async fn a_rectangle_without_pixels_draws_nothing() {
        let mut zlib = compressor();
        let bytes = update(&[
            rect(1, 1, 0, 3, 0, &[]),
            rect(1, 1, 3, 0, 5, &[]),
            zrle_rect(&mut zlib, 4, 4, 0, 0, &[]),
            rect(1, 1, 0, 3, 1, &[0, 0, 0, 0]),
            zrle_rect(&mut zlib, 0, 0, 1, 1, &[1, 10, 11, 12]),
        ]);
        let ops = decode(&mut sized(4, 4), &bytes).await.unwrap();
        assert_eq!(ops, screen(1, 1, [10]));
    }

    #[tokio::test]
    async fn an_update_that_ends_early_is_an_error() {
        let bytes = update(&[rect(0, 0, 2, 1, 0, &sent(10))]);
        for cut in [0, 2, 3, 10, 15, bytes.len() - 1] {
            let (ops, result) = decode_some(&mut sized(4, 4), &bytes[..cut]).await;
            assert!(matches!(result, Err(AppError::Io(_))), "{cut}");
            assert!(ops.is_empty(), "{cut}");
        }
    }

    #[tokio::test]
    async fn hextile_background_with_a_foreground_subrect() {
        let mut data = vec![0x02 | 0x04 | 0x08];
        data.extend(sent(10));
        data.extend(sent(20));
        data.extend([1, 0x10, 0x00]);
        let ops = decode(&mut sized(2, 2), &update(&[rect(0, 0, 2, 2, 5, &data)])).await;
        assert_eq!(ops.unwrap(), screen(2, 2, [10, 20, 10, 10]));
    }

    #[tokio::test]
    async fn hextile_raw_tile() {
        let mut data = vec![0x01];
        for n in [10, 20, 30, 40, 50, 60] {
            data.extend(sent(n));
        }
        let ops = decode(&mut sized(3, 2), &update(&[rect(0, 0, 3, 2, 5, &data)])).await;
        assert_eq!(ops.unwrap(), screen(3, 2, [10, 20, 30, 40, 50, 60]));
    }

    #[tokio::test]
    async fn hextile_coloured_subrects() {
        let mut data = vec![0x02 | 0x08 | 0x10];
        data.extend(sent(10));
        data.push(2);
        data.extend(sent(20));
        data.extend([0x00, 0x10]);
        data.extend(sent(30));
        data.extend([0x21, 0x12]);
        let ops = decode(&mut sized(4, 4), &update(&[rect(0, 0, 4, 4, 5, &data)])).await;
        let expected = [20, 20, 10, 10, 10, 10, 30, 30, 10, 10, 30, 30, 10, 10, 30, 30];
        assert_eq!(ops.unwrap(), screen(4, 4, expected));
    }

    #[tokio::test]
    async fn hextile_17x17_has_four_tiles() {
        let mut data = vec![0x02];
        data.extend(sent(10));
        // 1 wide: keeps the background, one subrect in a new foreground at its last row
        data.push(0x04 | 0x08);
        data.extend(sent(20));
        data.extend([1, 0x0F, 0x00]);
        // 1 high: a new background and the foreground of the tile before
        data.push(0x02 | 0x08);
        data.extend(sent(30));
        data.extend([1, 0xF0, 0x00]);
        data.push(0x01);
        data.extend(sent(40));
        let ops = decode(&mut sized(17, 17), &update(&[rect(0, 0, 17, 17, 5, &data)])).await;
        let expected = (0..17 * 17).map(|i| match (i % 17, i / 17) {
            (16, 16) => 40,
            (16, 15) | (15, 16) => 20,
            (_, 16) => 30,
            _ => 10,
        });
        assert_eq!(ops.unwrap(), screen(17, 17, expected));
    }

    #[tokio::test]
    async fn hextile_subrect_leaving_its_tile_is_invalid() {
        let subrect = |place: [u8; 2]| {
            let mut data = vec![0x02 | 0x04 | 0x08];
            data.extend(sent(10));
            data.extend(sent(20));
            data.push(1);
            data.extend(place);
            data
        };
        // x = 15, w = 2 and y = 15, h = 2 in a full tile
        for place in [[0xF0, 0x10], [0x0F, 0x01]] {
            let bytes = update(&[rect(0, 0, 16, 16, 5, &subrect(place))]);
            assert_invalid(&mut sized(32, 32), &bytes).await;
        }
        // x = 2, w = 2 and y = 2, h = 2 in a tile of 3 x 3
        for place in [[0x20, 0x10], [0x02, 0x01]] {
            let bytes = update(&[rect(0, 0, 3, 3, 5, &subrect(place))]);
            assert_invalid(&mut sized(32, 32), &bytes).await;
        }
        let bytes = update(&[rect(0, 0, 3, 3, 5, &subrect([0x11, 0x11]))]);
        let ops = decode(&mut sized(32, 32), &bytes).await.unwrap();
        assert_eq!(ops, [FrameOp::Rect {
            x: 0,
            y: 0,
            w: 3,
            h: 3,
            rgba: picture([10, 10, 10, 10, 20, 20, 10, 20, 20]),
        }]);
    }

    #[tokio::test]
    async fn zrle_solid_tile() {
        let ops = zrle_screen(3, 2, &[1, 10, 11, 12]).await.unwrap();
        assert_eq!(ops, screen(3, 2, [10; 6]));
    }

    #[tokio::test]
    async fn zrle_raw_tile() {
        let mut tiles = vec![0];
        for n in [10, 20, 30, 40] {
            tiles.extend(cpixel(n));
        }
        let ops = zrle_screen(2, 2, &tiles).await.unwrap();
        assert_eq!(ops, screen(2, 2, [10, 20, 30, 40]));
    }

    #[tokio::test]
    async fn zrle_packed_palette_of_two() {
        let mut tiles = vec![2];
        tiles.extend(cpixel(10));
        tiles.extend(cpixel(20));
        // 9 wide: a row is 9 bits in 2 bytes, the 7 padding bits are set here and ignored
        tiles.extend([0b1010_0101, 0b1111_1111, 0b0111_1111, 0b0111_1111]);
        let ops = zrle_screen(9, 2, &tiles).await.unwrap();
        let row0 = [20, 10, 20, 10, 10, 20, 10, 20, 20];
        let row1 = [10, 20, 20, 20, 20, 20, 20, 20, 10];
        assert_eq!(ops, screen(9, 2, row0.into_iter().chain(row1)));
    }

    #[tokio::test]
    async fn zrle_packed_palette_of_four() {
        let mut tiles = vec![4];
        for n in [10, 20, 30, 40] {
            tiles.extend(cpixel(n));
        }
        // 5 wide: 10 bits in 2 bytes
        tiles.extend([0b00_01_10_11, 0b00_111111, 0b11_10_01_00, 0b11_000000]);
        let ops = zrle_screen(5, 2, &tiles).await.unwrap();
        assert_eq!(ops, screen(5, 2, [10, 20, 30, 40, 10, 40, 30, 20, 10, 40]));
    }

    #[tokio::test]
    async fn zrle_packed_palette_of_sixteen() {
        let mut tiles = vec![16];
        for n in 0..16 {
            tiles.extend(cpixel(n * 10));
        }
        // 3 wide: 12 bits in 2 bytes
        tiles.extend([0xF0, 0x7F, 0x12, 0x30]);
        let ops = zrle_screen(3, 2, &tiles).await.unwrap();
        assert_eq!(ops, screen(3, 2, [150, 0, 70, 10, 20, 30]));
    }

    #[tokio::test]
    async fn zrle_packed_palette_sizes_between_the_powers_of_two() {
        let mut tiles = vec![3];
        for n in [10, 20, 30] {
            tiles.extend(cpixel(n));
        }
        tiles.push(0b10_01_00_10);
        assert_eq!(zrle_screen(4, 1, &tiles).await.unwrap(), screen(4, 1, [30, 20, 10, 30]));

        let mut tiles = vec![5];
        for n in [10, 20, 30, 40, 50] {
            tiles.extend(cpixel(n));
        }
        tiles.push(0x40);
        assert_eq!(zrle_screen(2, 1, &tiles).await.unwrap(), screen(2, 1, [50, 10]));
    }

    #[tokio::test]
    async fn zrle_palette_index_outside_the_palette_is_invalid() {
        let palette = |sub: u8, colours: u8| {
            let mut tiles = vec![sub];
            for n in 0..colours {
                tiles.extend(cpixel(n));
            }
            tiles
        };
        let packed_three = [palette(3, 3), vec![0b11_000000]].concat();
        assert_zrle_invalid(1, 1, &packed_three).await;
        let packed_five = [palette(5, 5), vec![0x50]].concat();
        assert_zrle_invalid(1, 1, &packed_five).await;
        let single = [palette(130, 2), vec![2]].concat();
        assert_zrle_invalid(1, 1, &single).await;
        let run = [palette(130, 2), vec![2 | 128, 0]].concat();
        assert_zrle_invalid(1, 1, &run).await;
    }

    #[tokio::test]
    async fn zrle_plain_rle_with_a_run_of_300() {
        let mut tiles = vec![128];
        tiles.extend(cpixel(10));
        tiles.extend([255, 44]);
        tiles.extend(cpixel(20));
        tiles.push(19);
        let ops = zrle_screen(64, 5, &tiles).await.unwrap();
        assert_eq!(ops, screen(64, 5, (0..320).map(|i| if i < 300 { 10 } else { 20 })));
    }

    #[tokio::test]
    async fn zrle_palette_rle_mixes_single_pixels_and_runs() {
        let mut tiles = vec![130];
        tiles.extend(cpixel(10));
        tiles.extend(cpixel(20));
        tiles.extend([0, 1 | 128, 2, 128, 1, 1, 1]);
        let ops = zrle_screen(4, 2, &tiles).await.unwrap();
        assert_eq!(ops, screen(4, 2, [10, 20, 20, 20, 10, 10, 20, 20]));
    }

    #[tokio::test]
    async fn zrle_palette_rle_with_the_largest_palette() {
        let mut tiles = vec![255];
        for n in 0..127 {
            tiles.extend(cpixel(n));
        }
        tiles.extend([126, 128, 0]);
        let ops = zrle_screen(2, 1, &tiles).await.unwrap();
        assert_eq!(ops, screen(2, 1, [126, 0]));
    }

    #[tokio::test]
    async fn zrle_65_pixels_are_two_tiles() {
        let tiles = [1, 10, 11, 12, 1, 20, 21, 22];
        let ops = zrle_screen(65, 1, &tiles).await.unwrap();
        assert_eq!(ops, screen(65, 1, (0..65).map(|i| if i < 64 { 10 } else { 20 })));
        let ops = zrle_screen(1, 65, &tiles).await.unwrap();
        assert_eq!(ops, screen(1, 65, (0..65).map(|i| if i < 64 { 10 } else { 20 })));
    }

    #[tokio::test]
    async fn zrle_tiles_go_left_to_right_then_down() {
        let mut tiles = Vec::new();
        for n in [10, 20, 30, 40] {
            tiles.push(1);
            tiles.extend(cpixel(n));
        }
        let ops = zrle_screen(65, 65, &tiles).await.unwrap();
        let expected = (0..65 * 65).map(|i| match (i % 65 < 64, i / 65 < 64) {
            (true, true) => 10,
            (false, true) => 20,
            (true, false) => 30,
            (false, false) => 40,
        });
        assert_eq!(ops, screen(65, 65, expected));
    }

    #[tokio::test]
    async fn zrle_rectangles_share_one_zlib_stream() {
        let mut zlib = compressor();
        let first = update(&[zrle_rect(&mut zlib, 0, 0, 2, 1, &[1, 10, 11, 12])]);
        let second = update(&[
            zrle_rect(&mut zlib, 1, 1, 2, 1, &[0, 20, 21, 22, 30, 31, 32]),
            zrle_rect(&mut zlib, 0, 3, 1, 1, &[1, 40, 41, 42]),
        ]);
        let mut decoder = sized(4, 4);
        let ops = decode(&mut decoder, &first).await.unwrap();
        assert_eq!(ops, [FrameOp::Rect { x: 0, y: 0, w: 2, h: 1, rgba: picture([10, 10]) }]);
        let ops = decode(&mut decoder, &second).await.unwrap();
        assert_eq!(
            ops,
            [
                FrameOp::Rect { x: 1, y: 1, w: 2, h: 1, rgba: picture([20, 30]) },
                FrameOp::Rect { x: 0, y: 3, w: 1, h: 1, rgba: picture([40]) },
            ],
        );
        // the second update only makes sense as the continuation of the first
        assert_invalid(&mut sized(4, 4), &second).await;
    }

    #[tokio::test]
    async fn zrle_unknown_subencodings_are_invalid() {
        assert_zrle_invalid(2, 2, &[&[127][..], &[0; 64]].concat()).await;
        // what packed pixels with 17 colours and runs with 1 colour would look like
        assert_zrle_invalid(2, 2, &[&[17][..], &[0; 17 * 3 + 2]].concat()).await;
        assert_zrle_invalid(2, 2, &[129, 10, 11, 12, 0, 0, 0, 0]).await;
    }

    #[tokio::test]
    async fn zrle_run_overflowing_the_tile_is_invalid() {
        assert_zrle_invalid(2, 2, &[128, 10, 11, 12, 4]).await;
        assert_zrle_invalid(2, 2, &[128, 10, 11, 12, 2, 20, 21, 22, 1]).await;
        assert_zrle_invalid(2, 2, &[130, 10, 11, 12, 20, 21, 22, 128, 4]).await;
        assert_zrle_invalid(2, 2, &[130, 10, 11, 12, 20, 21, 22, 0, 0, 0, 128, 1]).await;
        let endless = [&[128, 10, 11, 12][..], &[255; 5000]].concat();
        assert_zrle_invalid(64, 64, &endless).await;
    }

    #[tokio::test]
    async fn zrle_data_that_ends_early_is_invalid() {
        assert_zrle_invalid(2, 2, &[]).await;
        assert_zrle_invalid(2, 2, &[1, 10, 11]).await;
        assert_zrle_invalid(2, 2, &[0, 10, 11, 12, 20, 21, 22, 30, 31, 32]).await;
        assert_zrle_invalid(2, 2, &[2, 10, 11, 12, 20, 21, 22, 0]).await;
        assert_zrle_invalid(2, 2, &[128, 10, 11, 12, 2]).await;
        assert_zrle_invalid(2, 2, &[128, 10, 11, 12, 255]).await;
        assert_zrle_invalid(2, 2, &[130, 10, 11, 12, 20, 21, 22, 0, 1]).await;
        assert_zrle_invalid(65, 1, &[1, 10, 11, 12]).await;
        assert_invalid(&mut sized(2, 2), &update(&[zrle_data(0, 0, 2, 2, &[])])).await;
    }

    #[tokio::test]
    async fn zrle_truncated_zlib_data_is_invalid() {
        let tiles: Vec<u8> = [0].into_iter().chain((0..48).map(|i| i * 5)).collect();
        let data = deflate(&mut compressor(), &tiles);
        let whole = update(&[zrle_data(0, 0, 4, 4, &data)]);
        assert!(decode(&mut sized(4, 4), &whole).await.is_ok());
        for cut in [1, 2, data.len() / 2] {
            let bytes = update(&[zrle_data(0, 0, 4, 4, &data[..cut])]);
            assert_invalid(&mut sized(4, 4), &bytes).await;
        }
    }

    #[tokio::test]
    async fn zrle_data_beyond_the_tiles_is_invalid() {
        assert_zrle_invalid(2, 2, &[1, 10, 11, 12, 0]).await;
        let flood = [&[1, 10, 11, 12][..], &vec![0; 1 << 20]].concat();
        assert_zrle_invalid(2, 2, &flood).await;
        let mut zlib = compressor();
        let bytes = update(&[zrle_rect(&mut zlib, 0, 0, 0, 0, &[1, 10, 11, 12])]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
    }

    #[tokio::test]
    async fn zrle_corrupt_zlib_data_is_invalid() {
        let bytes = update(&[zrle_data(0, 0, 2, 2, &[0xFF; 16])]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
        let mut data = deflate(&mut compressor(), &[1, 10, 11, 12]);
        data.extend([0xFF; 16]);
        let bytes = update(&[zrle_data(0, 0, 2, 2, &data)]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
    }

    #[tokio::test]
    async fn zrle_stream_that_was_ended_cannot_go_on() {
        let mut ended = Vec::with_capacity(64);
        compressor().compress_vec(&[1, 10, 11, 12], &mut ended, FlushCompress::Finish).unwrap();
        let mut decoder = sized(2, 2);
        let bytes = update(&[zrle_data(0, 0, 2, 2, &ended)]);
        assert_eq!(decode(&mut decoder, &bytes).await.unwrap(), screen(2, 2, [10; 4]));
        let next = deflate(&mut compressor(), &[1, 20, 21, 22]);
        assert_invalid(&mut decoder, &update(&[zrle_data(0, 0, 2, 2, &next)])).await;

        let trailing = [&ended[..], &[0; 4]].concat();
        let bytes = update(&[zrle_data(0, 0, 2, 2, &trailing)]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
    }

    #[tokio::test]
    async fn zrle_declared_length_over_the_cap_is_invalid() {
        let bytes = update(&[rect(0, 0, 2, 2, 16, &((64 << 20) + 1u32).to_be_bytes())]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
        let bytes = update(&[rect(0, 0, 2, 2, 16, &u32::MAX.to_be_bytes())]);
        assert_invalid(&mut sized(2, 2), &bytes).await;
    }

    #[tokio::test]
    async fn desktop_size_resizes_and_later_rectangles_use_the_new_size() {
        let mut decoder = sized(4, 4);
        let bytes = update(&[
            rect(0, 0, 8, 2, -223, &[]),
            rect(6, 1, 2, 1, 0, &[sent(10), sent(20)].concat()),
        ]);
        let ops = decode(&mut decoder, &bytes).await.unwrap();
        assert_eq!(
            ops,
            [
                FrameOp::Resize { w: 8, h: 2 },
                FrameOp::Rect { x: 6, y: 1, w: 2, h: 1, rgba: picture([10, 20]) },
            ],
        );
        assert_eq!(decoder.size(), (8, 2));
        assert_invalid(&mut decoder, &update(&[rect(0, 3, 1, 1, 0, &sent(10))])).await;
    }

    #[tokio::test]
    async fn extended_desktop_size_resizes_when_the_status_is_zero() {
        let mut decoder = sized(4, 4);
        let screens = [&[2, 0, 0, 0][..], &[7; 32]].concat();
        let bytes = update(&[
            rect(1, 0, 8, 2, -308, &screens),
            rect(6, 1, 2, 1, 0, &[sent(10), sent(20)].concat()),
        ]);
        let ops = decode(&mut decoder, &bytes).await.unwrap();
        assert_eq!(
            ops,
            [
                FrameOp::Resize { w: 8, h: 2 },
                FrameOp::Rect { x: 6, y: 1, w: 2, h: 1, rgba: picture([10, 20]) },
            ],
        );
        assert_eq!(decoder.size(), (8, 2));
    }

    #[tokio::test]
    async fn extended_desktop_size_status_only_counts_in_a_reply() {
        let screens = [&[1, 0, 0, 0][..], &[7; 16]].concat();
        // reason 1 is the reply to a request of this client; only there the status is one
        let table = [
            (0, 0, true),
            (0, 3, true),
            (2, 1, true),
            (7, 9, true),
            (1, 0, true),
            (1, 1, false),
            (1, 3, false),
        ];
        for (reason, status, resized) in table {
            let mut decoder = sized(4, 4);
            let bytes = update(&[
                rect(reason, status, 8, 2, -308, &screens),
                rect(0, 1, 1, 1, 0, &sent(10)),
            ]);
            let mut ops = decode(&mut decoder, &bytes).await.unwrap();
            let drawn = FrameOp::Rect { x: 0, y: 1, w: 1, h: 1, rgba: picture([10]) };
            assert_eq!(ops.pop(), Some(drawn), "{reason} {status}");
            if resized {
                assert_eq!(ops, [FrameOp::Resize { w: 8, h: 2 }], "{reason} {status}");
                assert_eq!(decoder.size(), (8, 2));
            } else {
                assert!(ops.is_empty(), "{reason} {status}");
                assert_eq!(decoder.size(), (4, 4));
            }
        }
    }

    #[tokio::test]
    async fn extended_desktop_size_skips_every_screen() {
        let mut decoder = sized(4, 4);
        for (count, w) in [(0, 5), (255, 6)] {
            let screens = [&[count, 0, 0, 0][..], &vec![7; count as usize * 16]].concat();
            let bytes = update(&[rect(0, 0, w, 4, -308, &screens)]);
            let ops = decode(&mut decoder, &bytes).await.unwrap();
            assert_eq!(ops, [FrameOp::Resize { w, h: 4 }], "{count}");
        }
    }

    #[tokio::test]
    async fn the_size_the_screen_already_has_is_not_a_resize() {
        let mut decoder = sized(4, 4);
        let bytes = update(&[
            rect(0, 0, 4, 4, -308, &[1, 0, 0, 0, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7]),
            rect(0, 0, 4, 4, -223, &[]),
            rect(3, 3, 1, 1, 0, &sent(10)),
        ]);
        let ops = decode(&mut decoder, &bytes).await.unwrap();
        assert_eq!(ops, [FrameOp::Rect { x: 3, y: 3, w: 1, h: 1, rgba: picture([10]) }]);
        assert_eq!(decoder.size(), (4, 4));

        // a change in one direction only is a resize, and is announced once
        let bytes = update(&[rect(0, 0, 4, 5, -223, &[]), rect(0, 0, 4, 5, -223, &[])]);
        let ops = decode(&mut decoder, &bytes).await.unwrap();
        assert_eq!(ops, [FrameOp::Resize { w: 4, h: 5 }]);
    }

    #[tokio::test]
    async fn a_desktop_without_area_or_beyond_8k_is_refused() {
        let refused = |result: AppResult<()>| {
            matches!(result, Err(AppError::Internal(m))
                if m == "vnc: the server reported an invalid desktop size")
        };
        for (w, h) in [(0, 0), (0, 5), (5, 0), (8000, 6000), (8001, 5000)] {
            assert!(refused(Decoder::new(w, h).map(|_| ())), "{w}x{h}");
            let mut decoder = sized(4, 4);
            for resize in [rect(0, 0, w, h, -223, &[]), rect(0, 0, w, h, -308, &[0, 0, 0, 0])] {
                let (ops, result) = decode_some(&mut decoder, &update(&[resize])).await;
                assert!(refused(result), "{w}x{h}");
                assert!(ops.is_empty());
                assert_eq!(decoder.size(), (4, 4));
            }
        }
        for (w, h) in [(7680, 4320), (8000, 5000), (1, 65535)] {
            assert!(Decoder::new(w, h).is_ok(), "{w}x{h}");
            let bytes = update(&[rect(0, 0, w, h, -223, &[])]);
            let ops = decode(&mut sized(4, 4), &bytes).await.unwrap();
            assert_eq!(ops, [FrameOp::Resize { w, h }]);
        }
    }

    #[tokio::test]
    async fn cursor_takes_its_alpha_from_the_bitmask() {
        let mut data: Vec<u8> = (0..18).flat_map(|i| sent(i * 10)).collect();
        // pixels 0 and 8 of the first row; the 7 padding bits of the second row are set
        data.extend([0x80, 0x80, 0x00, 0x7F]);
        let ops = decode(&mut sized(4, 4), &update(&[rect(3, 1, 9, 2, -239, &data)])).await;
        let expected = (0..18).flat_map(|i| {
            let [r, g, b, _] = shown(i * 10);
            [r, g, b, if i == 0 || i == 8 { 255 } else { 0 }]
        });
        let cursor = FrameOp::Cursor { hot_x: 3, hot_y: 1, w: 9, h: 2, rgba: expected.collect() };
        assert_eq!(ops.unwrap(), [cursor]);
    }

    #[tokio::test]
    async fn an_empty_cursor_hides_it() {
        let ops = decode(&mut sized(4, 4), &update(&[rect(0, 0, 0, 0, -239, &[])])).await;
        let cursor = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
        assert_eq!(ops.unwrap(), [cursor]);
    }

    #[tokio::test]
    async fn a_cursor_without_area_hides_it() {
        for (w, h) in [(0, 5), (5, 0)] {
            let ops = decode(&mut sized(4, 4), &update(&[rect(3, 4, w, h, -239, &[])])).await;
            let cursor = FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 0, h: 0, rgba: Vec::new() };
            assert_eq!(ops.unwrap(), [cursor], "{w}x{h}");
        }
    }

    #[tokio::test]
    async fn cursor_hotspot_is_kept_inside_the_cursor() {
        for (hot, kept) in [((0, 2), (0, 1)), ((9, 0), (0, 0)), ((0, 1), (0, 1))] {
            let bytes = update(&[rect(hot.0, hot.1, 1, 2, -239, &[0xFF; 8 + 2])]);
            let ops = decode(&mut sized(4, 4), &bytes).await.unwrap();
            let cursor =
                FrameOp::Cursor { hot_x: kept.0, hot_y: kept.1, w: 1, h: 2, rgba: vec![0xFF; 8] };
            assert_eq!(ops, [cursor], "{hot:?}");
        }
    }

    #[tokio::test]
    async fn cursor_size_is_capped() {
        for (w, h) in [(2000, 2000), (1025, 1), (1, 1025)] {
            let bytes = update(&[rect(0, 0, w, h, -239, &[])]);
            assert_invalid(&mut sized(4, 4), &bytes).await;
        }
        let data = vec![0xFF; 1024 * 4 + 128];
        let bytes = update(&[rect(0, 0, 1024, 1, -239, &data)]);
        assert!(decode(&mut sized(4, 4), &bytes).await.is_ok());
    }

    #[tokio::test]
    async fn rectangles_outside_the_screen_are_invalid() {
        let mut zlib = compressor();
        let outside = [
            rect(9, 0, 2, 1, 0, &[sent(10), sent(20)].concat()),
            rect(0, 9, 1, 2, 0, &[sent(10), sent(20)].concat()),
            rect(10, 0, 1, 1, 0, &sent(10)),
            rect(65535, 65535, 65535, 65535, 0, &[]),
            rect(9, 0, 2, 1, 5, &[0x02, 10, 11, 12, 0]),
            zrle_rect(&mut zlib, 9, 0, 2, 1, &[1, 10, 11, 12]),
            zrle_rect(&mut zlib, 0, 9, 1, 2, &[1, 10, 11, 12]),
        ];
        for bytes in outside {
            assert_invalid(&mut sized(10, 10), &update(&[bytes])).await;
        }
        let bytes = update(&[rect(9, 9, 1, 1, 0, &sent(10))]);
        assert!(decode(&mut sized(10, 10), &bytes).await.is_ok());
    }

    #[tokio::test]
    async fn copy_rect_must_stay_on_the_screen_at_both_ends() {
        for (x, y, src_x, src_y) in [(0, 0, 7, 0), (0, 0, 0, 7), (7, 0, 0, 0), (0, 7, 0, 0)] {
            let bytes = update(&[rect(x, y, 4, 4, 1, &[0, src_x, 0, src_y])]);
            assert_invalid(&mut sized(10, 10), &bytes).await;
        }
        let bytes = update(&[rect(6, 6, 4, 4, 1, &[0, 0, 0, 6])]);
        assert!(decode(&mut sized(10, 10), &bytes).await.is_ok());
    }

    #[tokio::test]
    async fn operations_reach_the_sink_before_a_later_rectangle_fails() {
        let bytes = update(&[
            rect(0, 0, 1, 1, 0, &sent(10)),
            rect(1, 0, 1, 1, 1, &[0, 0, 0, 0]),
            rect(0, 0, 3, 3, -239, &[0; 36 + 3]),
            rect(4, 0, 1, 1, 0, &sent(20)),
            rect(2, 0, 1, 1, 0, &sent(30)),
        ]);
        let (ops, result) = decode_some(&mut sized(4, 4), &bytes).await;
        assert!(matches!(result, Err(AppError::Internal(m)) if m.ends_with("invalid update")));
        assert_eq!(
            ops,
            [
                FrameOp::Rect { x: 0, y: 0, w: 1, h: 1, rgba: picture([10]) },
                FrameOp::Copy { x: 1, y: 0, w: 1, h: 1, src_x: 0, src_y: 0 },
                FrameOp::Cursor { hot_x: 0, hot_y: 0, w: 3, h: 3, rgba: vec![0; 36] },
            ],
        );
    }

    // A 2 x 2 screen redrawn once in each of the four ways that count.
    fn four_screens(zlib: &mut Compress) -> Vec<Vec<u8>> {
        vec![
            rect(0, 0, 2, 2, 0, &[sent(10), sent(10), sent(10), sent(10)].concat()),
            rect(0, 0, 2, 2, 5, &[0x02, 10, 11, 12, 0]),
            zrle_rect(zlib, 0, 0, 2, 2, &[1, 10, 11, 12]),
            rect(0, 0, 2, 2, 1, &[0, 0, 0, 0]),
        ]
    }

    #[tokio::test]
    async fn an_update_may_redraw_the_screen_four_times_and_no_more() {
        let four = four_screens(&mut compressor());
        let ops = decode(&mut sized(2, 2), &update(&four)).await.unwrap();
        assert_eq!(ops.len(), 4);

        for encoding in [0, 5, 16, 1] {
            let mut zlib = compressor();
            let mut rects = four_screens(&mut zlib);
            rects.push(match encoding {
                0 => rect(1, 1, 1, 1, 0, &sent(20)),
                5 => rect(1, 1, 1, 1, 5, &[0x02, 20, 21, 22, 0]),
                16 => zrle_rect(&mut zlib, 1, 1, 1, 1, &[1, 20, 21, 22]),
                _ => rect(1, 1, 1, 1, 1, &[0, 0, 0, 0]),
            });
            let (ops, result) = decode_some(&mut sized(2, 2), &update(&rects)).await;
            assert!(
                matches!(result, Err(AppError::Internal(m)) if m.ends_with("invalid update")),
                "{encoding}"
            );
            assert_eq!(ops.len(), 4, "{encoding}");
        }
    }

    #[tokio::test]
    async fn a_resize_starts_the_count_again_and_cursors_are_not_counted() {
        for resize in [rect(0, 0, 4, 1, -223, &[]), rect(0, 0, 4, 1, -308, &[0, 0, 0, 0])] {
            let mut rects = four_screens(&mut compressor());
            rects.push(rect(0, 0, 2, 2, -239, &[0xFF; 16 + 2]));
            rects.push(resize.clone());
            rects.extend(vec![rect(0, 0, 4, 1, 1, &[0, 0, 0, 0]); 4]);
            rects.push(rect(0, 0, 4, 1, -239, &[0xFF; 16 + 1]));
            let ops = decode(&mut sized(2, 2), &update(&rects)).await.unwrap();
            assert_eq!(ops.len(), 4 + 1 + 1 + 4 + 1);

            // the size announced again is no resize, so the count goes on
            rects.insert(7, resize);
            rects.push(rect(3, 0, 1, 1, 1, &[0, 0, 0, 0]));
            let (ops, result) = decode_some(&mut sized(2, 2), &update(&rects)).await;
            assert!(matches!(result, Err(AppError::Internal(m)) if m.ends_with("invalid update")));
            assert_eq!(ops.len(), 4 + 1 + 1 + 4 + 1);
        }
    }

    #[tokio::test]
    async fn unknown_encoding_is_named() {
        let bytes = update(&[rect(0, 0, 1, 1, 7, &[])]);
        let refused = decode(&mut sized(4, 4), &bytes).await.unwrap_err();
        assert!(matches!(refused, AppError::Internal(m) if m == "vnc: unsupported encoding 7"));
    }

    fn noise(state: &mut u64) -> u8 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        (*state >> 32) as u8
    }

    fn run_length(data: &mut Vec<u8>, run: usize) {
        data.extend(std::iter::repeat_n(255, (run - 1) / 255));
        data.push(((run - 1) % 255) as u8);
    }

    // One ZRLE tile with made-up contents: its bytes, and the colour of each of its pixels.
    fn random_tile(state: &mut u64, sub: u8, tw: usize, pixels: usize) -> (Vec<u8>, Vec<u8>) {
        let palette: Vec<u8> = match sub {
            2..=16 => (0..sub).map(|_| noise(state) % 250).collect(),
            130..=255 => (0..sub - 128).map(|_| noise(state) % 250).collect(),
            _ => Vec::new(),
        };
        let mut data = vec![sub];
        data.extend(palette.iter().flat_map(|n| cpixel(*n)));
        let mut drawn = Vec::new();
        match sub {
            0 => {
                for _ in 0..pixels {
                    drawn.push(noise(state) % 250);
                    data.extend(cpixel(drawn[drawn.len() - 1]));
                }
            }
            1 => {
                drawn.resize(pixels, noise(state) % 250);
                data.extend(cpixel(drawn[0]));
            }
            2..=16 => {
                let bits = match sub {
                    2 => 1,
                    3 | 4 => 2,
                    _ => 4,
                };
                for _ in 0..pixels / tw {
                    let mut row = vec![0u8; (tw * bits + 7) / 8];
                    for col in 0..tw {
                        let index = noise(state) % sub;
                        row[col * bits / 8] |= index << (8 - bits - col * bits % 8);
                        drawn.push(palette[index as usize]);
                    }
                    data.extend(row);
                }
            }
            _ => {
                while drawn.len() < pixels {
                    let longest = (pixels - drawn.len()).min(700);
                    let run = match noise(state) % 3 {
                        0 => 1,
                        _ => 1 + (noise(state) as usize * 3) % longest,
                    };
                    let colour = if sub == 128 {
                        let colour = noise(state) % 250;
                        data.extend(cpixel(colour));
                        run_length(&mut data, run);
                        colour
                    } else {
                        let index = noise(state) % (sub - 128);
                        if run == 1 {
                            data.push(index);
                        } else {
                            data.push(index | 128);
                            run_length(&mut data, run);
                        }
                        palette[index as usize]
                    };
                    drawn.extend(std::iter::repeat_n(colour, run));
                }
            }
        }
        (data, drawn)
    }

    // The tiles of a w x h rectangle, all in one subencoding, and the picture they make.
    fn random_tiles(state: &mut u64, sub: u8, w: usize, h: usize) -> (Vec<u8>, Vec<u8>) {
        let (mut data, mut drawn) = (Vec::new(), vec![0u8; w * h]);
        for ty in (0..h).step_by(64) {
            for tx in (0..w).step_by(64) {
                let (tw, th) = ((w - tx).min(64), (h - ty).min(64));
                let (bytes, colours) = random_tile(state, sub, tw, tw * th);
                data.extend(bytes);
                for (i, colour) in colours.into_iter().enumerate() {
                    drawn[(ty + i / tw) * w + tx + i % tw] = colour;
                }
            }
        }
        (data, drawn)
    }

    const SUBENCODINGS: [u8; 12] = [0, 1, 2, 3, 4, 5, 15, 16, 128, 130, 131, 255];

    #[tokio::test]
    async fn zrle_draws_every_tile_width_in_every_subencoding() {
        let mut state = 0x2545_F491_4F6C_DD1D;
        for w in 1..=70 {
            for sub in SUBENCODINGS {
                let h = [1, 2, 3, 64, 65, 70][noise(&mut state) as usize % 6];
                let (data, drawn) = random_tiles(&mut state, sub, w, h);
                let ops = zrle_screen(w as u16, h as u16, &data).await.unwrap();
                assert!(ops == screen(w as u16, h as u16, drawn), "{w}x{h} in {sub}");
            }
        }
    }

    // Whatever the bytes are, decoding ends, and what it returns fits the screen.
    #[tokio::test]
    async fn noise_never_draws_outside_the_screen() {
        let mut state = 0x9E37_79B9_7F4A_7C15;
        for round in 0..6000 {
            let (w, h) = (1 + noise(&mut state) % 70, 1 + noise(&mut state) % 70);
            let (w, h) = (w as u16, h as u16);
            let len = noise(&mut state) as usize * 3;
            let mut body: Vec<u8> = (0..len).map(|_| noise(&mut state)).collect();
            let bytes = match round % 4 {
                0 => update(&[rect(0, 0, w, h, 5, &body)]),
                1 => {
                    // well-formed tiles with a few bytes changed
                    let sub = SUBENCODINGS[round / 4 % 12];
                    let (mut data, _) = random_tiles(&mut state, sub, w as usize, h as usize);
                    for change in body.chunks_exact(3).take(1 + round % 3) {
                        let at = (change[0] as usize * 256 + change[1] as usize) % data.len();
                        data[at] = change[2];
                    }
                    update(&[zrle_rect(&mut compressor(), 0, 0, w, h, &data)])
                }
                2 => {
                    // a subencoding that exists, so more than the first byte is looked at
                    body.insert(0, SUBENCODINGS[round / 4 % 12]);
                    update(&[zrle_rect(&mut compressor(), 0, 0, w, h, &body)])
                }
                _ => {
                    // several small rectangles in the encodings that exist, and one that does not
                    let rects: Vec<Vec<u8>> = body
                        .chunks_exact(24)
                        .map(|c| {
                            let encoding = [0, 1, 5, 16, -223, -239, -308, 7][c[0] as usize % 8];
                            let [x, y, w, h] = [c[1], c[2], c[3], c[4]].map(|v| (v % 12) as u16);
                            rect(x, y, w, h, encoding, &c[5..])
                        })
                        .collect();
                    update(&rects)
                }
            };
            let mut ops = Vec::new();
            let ended = sized(w, h).update(&mut &bytes[..], &mut |op| ops.push(op)).await;
            if let Err(refused) = ended {
                let expected = matches!(refused, AppError::Io(_) | AppError::Internal(_));
                assert!(expected, "round {round}");
            }
            let (mut width, mut height, mut drawn) = (w as usize, h as usize, 0);
            for op in ops {
                let fits = |x: u16, y: u16, w: u16, h: u16| {
                    x as usize + w as usize <= width && y as usize + h as usize <= height
                };
                match op {
                    FrameOp::Rect { x, y, w, h, rgba } => {
                        assert!(fits(x, y, w, h), "round {round}");
                        assert_eq!(rgba.len(), w as usize * h as usize * 4, "round {round}");
                        drawn += w as usize * h as usize;
                    }
                    FrameOp::Copy { x, y, w, h, src_x, src_y } => {
                        assert!(fits(x, y, w, h) && fits(src_x, src_y, w, h), "round {round}");
                        drawn += w as usize * h as usize;
                    }
                    FrameOp::Resize { w, h } => {
                        (width, height, drawn) = (w as usize, h as usize, 0);
                    }
                    FrameOp::Cursor { hot_x, hot_y, w, h, rgba } => {
                        let hidden = (hot_x, hot_y, w, h) == (0, 0, 0, 0);
                        assert!(hidden || (hot_x < w && hot_y < h), "round {round}");
                        assert_eq!(rgba.len(), w as usize * h as usize * 4, "round {round}");
                    }
                    FrameOp::Clipboard(_) | FrameOp::Closed(_) | FrameOp::Sync => {
                        panic!("round {round}")
                    }
                }
                assert!(drawn <= 4 * width * height, "round {round}");
            }
        }
    }
}
