use aes::cipher::{Block, BlockCipherEncrypt, KeyInit};
use aes::Aes128;
use num_bigint::BigUint;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::handshake::Login;
use super::{bounded, err};
use crate::error::{AppError, AppResult};

const MAX_KEY: usize = 1024;

fn left_padded(value: &BigUint, len: usize) -> Vec<u8> {
    let bytes = value.to_bytes_be();
    let mut out = vec![0u8; len - bytes.len()];
    out.extend(bytes);
    out
}

// Apple Remote Desktop login (RFB security type 30): a Diffie-Hellman exchange, then the
// username and password in one 128-byte block under AES-128-ECB with the MD5 of the shared
// secret as the key. The prime must not be zero.
pub fn response(
    generator: u16,
    prime: &[u8],
    server_public: &[u8],
    private: &[u8],
    pad: &[u8; 128],
    username: &str,
    password: &str,
) -> (Vec<u8>, Vec<u8>) {
    let modulus = BigUint::from_bytes_be(prime);
    let private = BigUint::from_bytes_be(private);
    let public = BigUint::from(generator).modpow(&private, &modulus);
    let shared = BigUint::from_bytes_be(server_public).modpow(&private, &modulus);
    let key = md5::compute(left_padded(&shared, prime.len()));

    let mut block = *pad;
    for (at, text) in [(0, username), (64, password)] {
        let text = &text.as_bytes()[..text.len().min(63)];
        block[at..at + text.len()].copy_from_slice(text);
        block[at + text.len()] = 0;
    }
    let cipher = Aes128::new_from_slice(&key.0).expect("16-byte AES key");
    for chunk in block.chunks_mut(16) {
        let mut b = Block::<Aes128>::try_from(&*chunk).expect("16-byte block");
        cipher.encrypt_block(&mut b);
        chunk.copy_from_slice(&b);
    }
    (block.to_vec(), left_padded(&public, prime.len()))
}

pub async fn login(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    login: &Login<'_>,
) -> AppResult<()> {
    let generator = stream.read_u16().await?;
    let key_len = bounded(stream.read_u16().await? as usize, MAX_KEY)?;
    let mut prime = vec![0u8; key_len];
    stream.read_exact(&mut prime).await?;
    let mut server_public = vec![0u8; key_len];
    stream.read_exact(&mut server_public).await?;
    if generator == 0 || prime.iter().all(|b| *b == 0) {
        return Err(err("the server sent an invalid key"));
    }

    let mut private = vec![0u8; key_len];
    getrandom::fill(&mut private).map_err(|_| AppError::Crypto)?;
    let mut pad = [0u8; 128];
    getrandom::fill(&mut pad).map_err(|_| AppError::Crypto)?;
    let (ciphertext, public) =
        response(generator, &prime, &server_public, &private, &pad, login.username, login.password);
    stream.write_all(&ciphertext).await?;
    stream.write_all(&public).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use aes::cipher::BlockCipherDecrypt;

    use super::*;

    // Produced with Python: pow() for the key exchange, hashlib.md5 and PyCryptodome's AES in
    // ECB mode, for a 512-bit prime from Crypto.Util.number.getPrime, generator 2, pad 0..127,
    // username "faye" and password "hunter2".
    const PRIME: &str = concat!(
        "b5e83072619159ab20f39481ab0f417865a6a7a516b112988e39d1b2219c277c",
        "9e8da0345e212b6dc39482419ba5c51d48ed69d12f21f1f5d4754140b2fc8417",
    );
    const CLIENT_PRIVATE: &str = concat!(
        "01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3da",
        "e1e8eff6fd040b121920272e353c434a51585f666d747b828990979ea5acb3ba",
    );
    const SERVER_PRIVATE: &str = concat!(
        "05121f2c394653606d7a8794a1aebbc8d5e2effc091623303d4a5764717e8b98",
        "a5b2bfccd9e6f3000d1a2734414e5b6875828f9ca9b6c3d0ddeaf704111e2b38",
    );
    const SERVER_PUBLIC: &str = concat!(
        "5273a51d8f589aa1f25129f4ab6e6c1026426e03c6bc354f0e7e8ff1b87f542f",
        "54c1232a30b0c593ce2c2bc8f526310280834cc4407ba5246cd6624fc780e938",
    );
    const CLIENT_PUBLIC: &str = concat!(
        "57f2a1e25c8cdb162efb293474e21f3578921844b3ff5a3a2f5c0c8efedbe0a2",
        "6713252ea239ae4040d213a47e2dcbccb29be2e9c4ce8fbb0d54995dd6b6594f",
    );
    const CIPHERTEXT: &str = concat!(
        "f75bf02169fcbab3a623b7e485212d758038e1a1c4ce06de0b96794f1d78ff85",
        "954c3984010a1e48c80a9c1cb0822795e68efad5868987794a5f5491ba51a1da",
        "53b0577e03fb6111cee9a3e738a716996caf73b342bccc8d03f54de315f60e2d",
        "ae7ccfcd45d1e621f9c69d76692a7b1923c962237fa40c305cab4e71cb065393",
    );
    // The same login with generator 5 and the 32-bit prime 4294967291 in an 8-byte key, client
    // private key 123456789abcdef0: every value is shorter than the key.
    const SMALL_CIPHERTEXT: &str = concat!(
        "826b2ca991753ebba4fd26d90b258d8c15a655de3eeac85340809ffcd2a70470",
        "36c854b58ee76c5ea6c0dcd462bdf5826709f0c7c9b64e8271722be44f4264f9",
        "eb4b6fd7b7205bb6eb6c18c16f3d45b8980ef82bcfcef8ac05d427f7a190719a",
        "860eb0a3554f1ca4388fdc11ce893c00ede0e50875804e707d3d411f1e59ccc7",
    );

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn pad() -> [u8; 128] {
        std::array::from_fn(|i| i as u8)
    }

    // The server's half of the exchange: its own shared secret, then the plaintext.
    fn server_reads(server_private: &str, client_public: &[u8], ciphertext: &[u8]) -> Vec<u8> {
        let prime = hex(PRIME);
        let shared = BigUint::from_bytes_be(client_public)
            .modpow(&BigUint::from_bytes_be(&hex(server_private)), &BigUint::from_bytes_be(&prime))
            .to_bytes_be();
        let mut padded = vec![0u8; prime.len() - shared.len()];
        padded.extend(shared);
        let cipher = Aes128::new_from_slice(&md5::compute(padded).0).unwrap();
        let mut plain = ciphertext.to_vec();
        for block in plain.chunks_mut(16) {
            let mut b = Block::<Aes128>::try_from(&*block).unwrap();
            cipher.decrypt_block(&mut b);
            block.copy_from_slice(&b);
        }
        plain
    }

    fn vector_response(username: &str, password: &str) -> (Vec<u8>, Vec<u8>) {
        let (prime, server_public) = (hex(PRIME), hex(SERVER_PUBLIC));
        response(2, &prime, &server_public, &hex(CLIENT_PRIVATE), &pad(), username, password)
    }

    // What the server has received once `login` is through with the parameters it was sent.
    async fn play(params: Vec<u8>, username: &str, password: &str) -> (AppResult<()>, Vec<u8>) {
        let (mut client, mut server) = tokio::io::duplex(4096);
        server.write_all(&params).await.unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            let result = login(&mut client, &Login { username, password }).await;
            drop(client);
            let mut sent = Vec::new();
            server.read_to_end(&mut sent).await.unwrap();
            (result, sent)
        })
        .await
        .expect("login hung")
    }

    fn params(generator: u16, key_len: u16, prime: &[u8], server_public: &[u8]) -> Vec<u8> {
        [&generator.to_be_bytes()[..], &key_len.to_be_bytes(), prime, server_public].concat()
    }

    #[test]
    fn matches_an_independent_implementation() {
        let (ciphertext, public) = vector_response("faye", "hunter2");
        assert_eq!(ciphertext, hex(CIPHERTEXT));
        assert_eq!(public, hex(CLIENT_PUBLIC));
    }

    #[test]
    fn short_values_are_left_padded_to_the_key_length() {
        let prime = hex("00000000fffffffb");
        let server_public = hex("000000002620595a");
        let private = hex("123456789abcdef0");
        let (ciphertext, public) =
            response(5, &prime, &server_public, &private, &pad(), "faye", "hunter2");
        assert_eq!(public, hex("0000000047e17aa6"));
        assert_eq!(ciphertext, hex(SMALL_CIPHERTEXT));
    }

    #[test]
    fn the_server_derives_the_same_key_and_reads_the_login() {
        let (ciphertext, public) = vector_response("faye", "hunter2");
        let plain = server_reads(SERVER_PRIVATE, &public, &ciphertext);
        assert_eq!(&plain[..5], b"faye\0");
        assert_eq!(&plain[5..64], &pad()[5..64]);
        assert_eq!(&plain[64..72], b"hunter2\0");
        assert_eq!(&plain[72..], &pad()[72..]);
    }

    #[test]
    fn a_long_username_or_password_is_cut_at_63_bytes() {
        let username: String = ('a'..='z').cycle().take(100).collect();
        let password: String = ('0'..='9').cycle().take(64).collect();
        let (ciphertext, public) = vector_response(&username, &password);
        let plain = server_reads(SERVER_PRIVATE, &public, &ciphertext);
        assert_eq!(&plain[..63], &username.as_bytes()[..63]);
        assert_eq!(plain[63], 0);
        assert_eq!(&plain[64..127], &password.as_bytes()[..63]);
        assert_eq!(plain[127], 0);
    }

    #[test]
    fn an_empty_username_is_sent_as_empty() {
        let (ciphertext, public) = vector_response("", "hunter2");
        let plain = server_reads(SERVER_PRIVATE, &public, &ciphertext);
        assert_eq!(plain[0], 0);
        assert_eq!(&plain[1..64], &pad()[1..64]);
        assert_eq!(&plain[64..72], b"hunter2\0");
    }

    #[tokio::test]
    async fn login_writes_the_ciphertext_and_the_public_key() {
        let (prime, server_public) = (hex(PRIME), hex(SERVER_PUBLIC));
        let (result, sent) = play(params(2, 64, &prime, &server_public), "faye", "hunter2").await;
        result.unwrap();
        assert_eq!(sent.len(), 128 + 64);
        let plain = server_reads(SERVER_PRIVATE, &sent[128..], &sent[..128]);
        assert_eq!(&plain[..5], b"faye\0");
        assert_eq!(&plain[64..72], b"hunter2\0");
    }

    #[tokio::test]
    async fn every_login_uses_a_fresh_key_and_pad() {
        let (prime, server_public) = (hex(PRIME), hex(SERVER_PUBLIC));
        let (_, first) = play(params(2, 64, &prime, &server_public), "faye", "hunter2").await;
        let (_, second) = play(params(2, 64, &prime, &server_public), "faye", "hunter2").await;
        assert_ne!(first[128..], second[128..]);
        let plain = |sent: &[u8]| server_reads(SERVER_PRIVATE, &sent[128..], &sent[..128]);
        assert_ne!(plain(&first)[5..64], plain(&second)[5..64]);
    }

    #[tokio::test]
    async fn a_key_length_of_zero_or_2000_is_refused() {
        let (result, sent) = play(params(2, 0, &[], &[]), "faye", "hunter2").await;
        assert!(result.unwrap_err().to_string().contains("the server sent an invalid key"));
        assert!(sent.is_empty());

        for key_len in [1025, 2000] {
            let (result, sent) = play(params(2, key_len, &[], &[]), "faye", "hunter2").await;
            let refused = result.unwrap_err().to_string();
            assert!(refused.contains(&format!("declared length {key_len} exceeds 1024")));
            assert!(sent.is_empty());
        }
    }

    #[tokio::test]
    async fn a_zero_generator_or_prime_is_refused() {
        let (prime, server_public) = (hex(PRIME), hex(SERVER_PUBLIC));
        let zero_generator = params(0, 64, &prime, &server_public);
        let zero_prime = params(2, 64, &[0; 64], &server_public);
        for case in [zero_generator, zero_prime] {
            let (result, sent) = play(case, "faye", "hunter2").await;
            assert!(result.unwrap_err().to_string().contains("the server sent an invalid key"));
            assert!(sent.is_empty());
        }
    }

    #[tokio::test]
    async fn odd_keys_still_get_an_answer() {
        let keys = [([0, 1], [0, 0]), ([0, 2], [0, 9]), ([0, 4], [255, 255]), ([255, 255], [0, 0])];
        for (prime, server_public) in keys {
            let (result, sent) = play(params(7, 2, &prime, &server_public), "", "hunter2").await;
            result.unwrap();
            assert_eq!(sent.len(), 128 + 2, "{prime:?}");
        }
    }
}
