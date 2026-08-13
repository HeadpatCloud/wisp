use std::net::Ipv6Addr;

// `[::1]` is the URL form of an address, not an address. Windows' getaddrinfo accepts it,
// glibc doesn't - stripping the brackets makes the same profile work on every platform.
pub fn normalize_host(host: &str) -> String {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .filter(|h| h.parse::<Ipv6Addr>().is_ok())
        .unwrap_or(host)
        .to_string()
}

// For anything parsed as a SocketAddr or used as a URL authority, a bare IPv6 literal has
// to be bracketed or its own colons read as the port.
pub fn authority(host: &str, port: u16) -> String {
    let host = normalize_host(host);
    if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

// The authority without a port, for URLs that leave the default implicit.
pub fn url_host(host: &str) -> String {
    let host = normalize_host(host);
    if host.parse::<Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_brackets_from_ipv6_only() {
        assert_eq!(normalize_host("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(normalize_host("2001:db8::1"), "2001:db8::1");
        assert_eq!(normalize_host("example.com"), "example.com");
        assert_eq!(normalize_host("10.0.0.1"), "10.0.0.1");
        // Not a valid literal, so it is left exactly as typed.
        assert_eq!(normalize_host("[not-an-addr]"), "[not-an-addr]");
    }

    #[test]
    fn authority_brackets_ipv6_and_parses_as_a_socket_addr() {
        use std::net::SocketAddr;
        assert_eq!(authority("2001:db8::1", 22), "[2001:db8::1]:22");
        assert_eq!(authority("[2001:db8::1]", 22), "[2001:db8::1]:22");
        assert_eq!(authority("example.com", 21), "example.com:21");
        assert_eq!(authority("10.0.0.1", 5900), "10.0.0.1:5900");

        assert!(authority("::1", 8080).parse::<SocketAddr>().is_ok());
        assert!(authority("127.0.0.1", 8080).parse::<SocketAddr>().is_ok());
    }

    #[test]
    fn url_host_brackets_ipv6_only() {
        assert_eq!(url_host("2001:db8::1"), "[2001:db8::1]");
        assert_eq!(url_host("[2001:db8::1]"), "[2001:db8::1]");
        assert_eq!(url_host("s3.example.com"), "s3.example.com");
    }
}
