use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{AppError, AppResult};

// Minimal SOCKS5: negotiate no-auth, parse a CONNECT request, return the target.
// Does NOT send the final success reply - the caller does that AFTER opening the
// SSH channel (reply 0x00 on success, 0x05 on failure).
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S) -> AppResult<(String, u16)> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head).await?;
    if head[0] != 0x05 {
        return Err(AppError::Tunnel("not socks5".into()));
    }
    let nmethods = head[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0x00) {
        stream.write_all(&[0x05, 0xFF]).await?;
        return Err(AppError::Tunnel("no no-auth method".into()));
    }
    stream.write_all(&[0x05, 0x00]).await?;

    let mut req = [0u8; 4];
    stream.read_exact(&mut req).await?;
    if req[0] != 0x05 {
        return Err(AppError::Tunnel("bad request version".into()));
    }
    if req[1] != 0x01 {
        reply(stream, 0x07).await?; // command not supported
        return Err(AppError::Tunnel("only CONNECT supported".into()));
    }
    let host = match req[3] {
        0x01 => {
            let mut a = [0u8; 4];
            stream.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        0x03 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            let mut d = vec![0u8; len[0] as usize];
            stream.read_exact(&mut d).await?;
            String::from_utf8(d).map_err(|_| AppError::Tunnel("bad domain".into()))?
        }
        0x04 => {
            let mut a = [0u8; 16];
            stream.read_exact(&mut a).await?;
            std::net::Ipv6Addr::from(a).to_string()
        }
        _ => {
            reply(stream, 0x08).await?; // address type not supported
            return Err(AppError::Tunnel("bad address type".into()));
        }
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).await?;
    Ok((host, u16::from_be_bytes(port)))
}

// REP: 0x00 success, 0x05 connection refused, 0x07 cmd unsupported, 0x08 atyp unsupported.
pub async fn reply<S: AsyncWrite + Unpin>(stream: &mut S, rep: u8) -> std::io::Result<()> {
    stream.write_all(&[0x05, rep, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn parses_domain_connect() {
        let (mut client, mut server) = tokio::io::duplex(256);
        // req[0..3] = greeting placeholder (skipped); req[3..] = the CONNECT request:
        // ver=5, CONNECT, rsv, atyp=domain(0x03), len=9, "localhost", port=22
        let mut req = vec![0x05u8, 0x01, 0x00, 0x05, 0x01, 0x00, 0x03, 0x09];
        req.extend_from_slice(b"localhost");
        req.extend_from_slice(&22u16.to_be_bytes());
        // write the greeting (3 bytes) then the request part after reading the method reply
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            client.write_all(&[0x05, 0x01, 0x00]).await.unwrap();
            let mut method_reply = [0u8; 2];
            client.read_exact(&mut method_reply).await.unwrap();
            assert_eq!(method_reply, [0x05, 0x00]);
            client.write_all(&req[3..]).await.unwrap(); // the CONNECT request bytes
        });
        let (host, port) = handshake(&mut server).await.unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 22);
    }

    #[tokio::test]
    async fn rejects_non_socks5() {
        let (mut client, mut server) = tokio::io::duplex(16);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            client.write_all(&[0x04, 0x01]).await.unwrap();
        });
        assert!(matches!(handshake(&mut server).await, Err(AppError::Tunnel(_))));
    }
}
