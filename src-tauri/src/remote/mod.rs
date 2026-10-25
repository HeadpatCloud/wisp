pub mod frame;
pub mod tls;
pub mod trust;

use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

pub use frame::{FrameBytes, FrameOp};

const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

pub fn keep_alive(tcp: &TcpStream) -> std::io::Result<()> {
    // Windows sends 10 probes unless told otherwise: 130 s for a link that is gone.
    let keepalive = TcpKeepalive::new()
        .with_time(KEEPALIVE_IDLE)
        .with_interval(KEEPALIVE_INTERVAL)
        .with_retries(2);
    SockRef::from(tcp).set_tcp_keepalive(&keepalive)
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    // A server that is gone without a word (a VM turned off) leaves only the keepalive to notice.
    #[tokio::test]
    async fn the_keepalive_gives_up_on_a_dead_link_within_a_minute() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let tcp = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        keep_alive(&tcp).unwrap();
        // socket2 has no getter for the other two on Windows.
        let probes = SockRef::from(&tcp).tcp_keepalive_retries().unwrap();
        let limit = KEEPALIVE_IDLE + KEEPALIVE_INTERVAL * probes;
        assert!(limit < Duration::from_secs(60), "{probes} probes: {limit:?}");
    }
}
