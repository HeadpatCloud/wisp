#[derive(Debug, PartialEq, Eq)]
pub struct SshConfigHost {
    pub name: String,
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_file: Option<String>,
    pub proxy_jump: Option<String>,
}

// Parse an OpenSSH client config into per-Host entries. Keywords are
// case-insensitive; `Host` patterns that are wildcards (contain * or ?) are
// skipped (they are defaults, not connectable hosts). Only the first
// IdentityFile / first ProxyJump token is kept.
pub fn parse_ssh_config(text: &str) -> Vec<SshConfigHost> {
    let mut hosts = Vec::new();
    let mut current: Option<SshConfigHost> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = line.split_once(|c: char| c.is_whitespace() || c == '=') else {
            continue;
        };
        let key = key.to_lowercase();
        let value = rest.trim().trim_start_matches('=').trim();
        if key == "host" {
            if let Some(h) = current.take() {
                hosts.push(h);
            }
            let pattern = value.split_whitespace().next().unwrap_or("");
            if pattern.is_empty() || pattern.contains('*') || pattern.contains('?') {
                current = None; // wildcard/default block - consume but do not import
                continue;
            }
            current = Some(SshConfigHost {
                name: pattern.to_string(),
                host_name: None,
                port: None,
                user: None,
                identity_file: None,
                proxy_jump: None,
            });
        } else if let Some(h) = current.as_mut() {
            match key.as_str() {
                "hostname" => h.host_name = Some(value.to_string()),
                "port" => h.port = value.parse().ok(),
                "user" => h.user = Some(value.to_string()),
                "identityfile" => {
                    if h.identity_file.is_none() {
                        h.identity_file = Some(value.to_string());
                    }
                }
                "proxyjump" => {
                    if h.proxy_jump.is_none() {
                        let token = value.split_whitespace().next().unwrap_or(value);
                        // A comma chain is true multi-hop, which a single jumpHostId
                        // can't represent - leave it unmatched rather than mislink.
                        let alias = if token.contains(',') {
                            token.to_string()
                        } else {
                            let host = token.rsplit('@').next().unwrap_or(token);
                            // Only strip a :port from a plain host:port; leave bracketed
                            // and bare IPv6 literals alone.
                            if host.starts_with('[') || host.matches(':').count() != 1 {
                                host.to_string()
                            } else {
                                host.split(':').next().unwrap_or(host).to_string()
                            }
                        };
                        h.proxy_jump = Some(alias);
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(h) = current.take() {
        hosts.push(h);
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_host_all_fields() {
        let text = "Host web\n  HostName 1.2.3.4\n  Port 2222\n  User me\n  IdentityFile ~/.ssh/id_ed25519\n  ProxyJump bastion\n";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.len(), 1);
        let h = &hosts[0];
        assert_eq!(h.name, "web");
        assert_eq!(h.host_name.as_deref(), Some("1.2.3.4"));
        assert_eq!(h.port, Some(2222));
        assert_eq!(h.user.as_deref(), Some("me"));
        assert_eq!(h.identity_file.as_deref(), Some("~/.ssh/id_ed25519"));
        assert_eq!(h.proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn minimal_host_has_defaults_none() {
        let hosts = parse_ssh_config("Host db\nHost web\n  HostName w\n");
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].name, "db");
        assert_eq!(hosts[0].host_name, None);
        assert_eq!(hosts[0].port, None);
        assert_eq!(hosts[1].host_name.as_deref(), Some("w"));
    }

    #[test]
    fn proxy_jump_strips_user_and_port() {
        let hosts = parse_ssh_config("Host web\n  HostName w\n  ProxyJump me@bastion:2222\n");
        assert_eq!(hosts[0].proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn proxy_jump_leaves_ipv6_literal_intact() {
        let hosts = parse_ssh_config("Host web\n  HostName w\n  ProxyJump [2001:db8::1]:22\n");
        assert_eq!(hosts[0].proxy_jump.as_deref(), Some("[2001:db8::1]:22"));
    }

    #[test]
    fn skips_wildcard_blocks() {
        let text = "Host *\n  ForwardAgent yes\nHost real\n  HostName r\n";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["real"]);
    }

    #[test]
    fn keywords_case_insensitive_and_equals() {
        let text = "host x\n  HOSTNAME=h\n  port = 2200\n";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts[0].host_name.as_deref(), Some("h"));
        assert_eq!(hosts[0].port, Some(2200));
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let text = "# a comment\n\nHost y\n  # inner\n  User u\n";
        let hosts = parse_ssh_config(text);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user.as_deref(), Some("u"));
    }
}
