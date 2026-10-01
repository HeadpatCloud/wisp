use des::cipher::{Block, BlockCipherEncrypt, KeyInit};
use des::Des;

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
    fn auth_response_matches_known_answer() {
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        // Expected bytes from Node's OpenSSL des-ecb with each key byte bit-reversed, no padding.
        assert_eq!(
            vnc_auth_response("password", &challenge),
            [
                0xb8, 0x66, 0x92, 0x41, 0x25, 0xc8, 0xee, 0xbb, 0x9d, 0xeb, 0xc1, 0xdb, 0x61, 0xc5,
                0x38, 0xe2,
            ],
        );
    }

    // Expected bytes from PyCryptodome DES-ECB over the bit-reversed key bytes.
    #[test]
    fn auth_response_uses_the_first_eight_password_bytes() {
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            vnc_auth_response("password123", &challenge),
            [
                0xb8, 0x66, 0x92, 0x41, 0x25, 0xc8, 0xee, 0xbb, 0x9d, 0xeb, 0xc1, 0xdb, 0x61, 0xc5,
                0x38, 0xe2,
            ],
        );
    }

    #[test]
    fn auth_response_pads_a_short_password_with_zeros() {
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            vnc_auth_response("pw", &challenge),
            [
                0x85, 0x86, 0x00, 0xd9, 0xaf, 0x14, 0x3c, 0x9e, 0x65, 0x41, 0xd3, 0xdd, 0x92, 0xa8,
                0x35, 0xd0,
            ],
        );
    }

    #[test]
    fn input_events_have_correct_layout() {
        assert_eq!(pointer_event(0b10, 0x0102, 0x0304), [5, 2, 1, 2, 3, 4]);
        assert_eq!(key_event(true, 0x0041), [4, 1, 0, 0, 0, 0, 0, 0x41]);
        assert_eq!(fb_update_request(true, 0, 0, 0x0102, 0x0304), [3, 1, 0, 0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(client_cut_text("hi"), [6, 0, 0, 0, 0, 0, 0, 2, b'h', b'i']);
    }
}
