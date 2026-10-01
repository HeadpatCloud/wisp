use specta::Type;
use tauri::ipc::{InvokeResponseBody, IpcResponse};

pub enum FrameOp {
    Rect { x: u16, y: u16, w: u16, h: u16, rgba: Vec<u8> },
    Copy { x: u16, y: u16, w: u16, h: u16, src_x: u16, src_y: u16 },
    Resize { w: u16, h: u16 },
    Cursor { hot_x: u16, hot_y: u16, w: u16, h: u16, rgba: Vec<u8> },
    Clipboard(String),
    Closed(String),
}

impl FrameOp {
    pub fn encode(self) -> FrameBytes {
        let (kind, fields, tail) = match self {
            Self::Rect { x, y, w, h, rgba } => (1, vec![x, y, w, h], rgba),
            Self::Copy { x, y, w, h, src_x, src_y } => {
                (2, vec![x, y, w, h, src_x, src_y], Vec::new())
            }
            Self::Resize { w, h } => (3, vec![w, h], Vec::new()),
            Self::Cursor { hot_x, hot_y, w, h, rgba } => (4, vec![hot_x, hot_y, w, h], rgba),
            Self::Clipboard(text) => (5, Vec::new(), text.into_bytes()),
            Self::Closed(reason) => (6, Vec::new(), reason.into_bytes()),
        };
        let mut out = Vec::with_capacity(1 + fields.len() * 2 + tail.len());
        out.push(kind);
        for field in fields {
            out.extend_from_slice(&field.to_be_bytes());
        }
        out.extend_from_slice(&tail);
        FrameBytes(out)
    }
}

// The bindings type this as number[], but it is sent raw and arrives as an ArrayBuffer.
#[derive(Type)]
pub struct FrameBytes(pub Vec<u8>);

impl IpcResponse for FrameBytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rect() {
        let op = FrameOp::Rect { x: 1, y: 2, w: 1, h: 1, rgba: vec![9, 8, 7, 255] };
        assert_eq!(op.encode().0, [1, 0, 1, 0, 2, 0, 1, 0, 1, 9, 8, 7, 255]);
    }

    #[test]
    fn encodes_copy() {
        let op = FrameOp::Copy { x: 1, y: 2, w: 3, h: 4, src_x: 5, src_y: 6 };
        assert_eq!(op.encode().0, [2, 0, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6]);
    }

    #[test]
    fn encodes_resize() {
        let op = FrameOp::Resize { w: 0x0780, h: 0x0438 };
        assert_eq!(op.encode().0, [3, 7, 0x80, 4, 0x38]);
    }

    #[test]
    fn encodes_cursor() {
        let op = FrameOp::Cursor { hot_x: 1, hot_y: 2, w: 1, h: 1, rgba: vec![1, 2, 3, 4] };
        assert_eq!(op.encode().0, [4, 0, 1, 0, 2, 0, 1, 0, 1, 1, 2, 3, 4]);
    }

    #[test]
    fn encodes_clipboard() {
        let op = FrameOp::Clipboard("hé".to_string());
        assert_eq!(op.encode().0, [5, b'h', 0xC3, 0xA9]);
    }

    #[test]
    fn encodes_closed() {
        let op = FrameOp::Closed("bye".to_string());
        assert_eq!(op.encode().0, [6, b'b', b'y', b'e']);
    }
}
