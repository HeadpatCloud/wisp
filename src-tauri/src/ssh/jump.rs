use std::collections::HashSet;

use crate::error::{AppError, AppResult};
use crate::store::hosts::SshTarget;

// Walk jumpHostId from the target back to a root, detecting cycles and missing
// bastions. Returned chain is ordered root-first, target-last.
pub fn resolve_jump_chain(
    target: impl Fn(&str) -> AppResult<SshTarget>,
    target_id: &str,
) -> AppResult<Vec<SshTarget>> {
    let mut chain: Vec<SshTarget> = Vec::new();
    let mut seen = HashSet::new();
    let mut current = target_id.to_string();
    loop {
        if !seen.insert(current.clone()) {
            return Err(AppError::Ssh(format!("jump host cycle at profile {current}")));
        }
        let hop = match target(&current) {
            Err(AppError::NotFound(_)) if !chain.is_empty() => {
                return Err(AppError::Ssh(
                    "Its jump host no longer exists. Pick another in the host editor.".into(),
                ));
            }
            hop => hop?,
        };
        let next = hop.jump_host_id.clone();
        chain.push(hop);
        match next {
            Some(j) => current = j,
            None => break,
        }
    }
    chain.reverse();
    Ok(chain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::model::AuthMethod;

    fn target(id: &str, jump: Option<&str>) -> SshTarget {
        SshTarget {
            id: id.into(),
            host: format!("{id}.example"),
            port: 22,
            username: "me".into(),
            auth_method: AuthMethod::Agent,
            keys: vec![],
            secret_id: None,
            jump_host_id: jump.map(|s| s.to_string()),
        }
    }

    fn lookup(all: Vec<SshTarget>) -> impl Fn(&str) -> AppResult<SshTarget> {
        move |id| {
            all.iter()
                .find(|t| t.id == id)
                .cloned()
                .ok_or_else(|| AppError::NotFound("This connection no longer exists.".into()))
        }
    }

    fn ids(chain: &[SshTarget]) -> Vec<&str> {
        chain.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn no_jump_is_single_element() {
        let chain = resolve_jump_chain(lookup(vec![target("t", None)]), "t").unwrap();
        assert_eq!(ids(&chain), ["t"]);
    }

    #[test]
    fn two_hops_are_root_first() {
        let all = vec![target("t", Some("b")), target("b", None)];
        assert_eq!(ids(&resolve_jump_chain(lookup(all), "t").unwrap()), ["b", "t"]);
    }

    #[test]
    fn three_hops_chain_in_order() {
        let all = vec![target("t", Some("b")), target("b", Some("a")), target("a", None)];
        assert_eq!(ids(&resolve_jump_chain(lookup(all), "t").unwrap()), ["a", "b", "t"]);
    }

    #[test]
    fn cycle_is_rejected() {
        let all = vec![target("t", Some("b")), target("b", Some("t"))];
        assert!(matches!(resolve_jump_chain(lookup(all), "t"), Err(AppError::Ssh(_))));
    }

    #[test]
    fn missing_jump_host_names_the_fix() {
        let all = vec![target("t", Some("ghost"))];
        assert!(matches!(
            resolve_jump_chain(lookup(all), "t"),
            Err(AppError::Ssh(m)) if m == "Its jump host no longer exists. Pick another in the host editor."
        ));
    }

    #[test]
    fn a_missing_target_is_not_found() {
        assert!(matches!(resolve_jump_chain(lookup(vec![]), "t"), Err(AppError::NotFound(_))));
    }
}
