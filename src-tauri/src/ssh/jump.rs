use std::collections::HashSet;

use crate::error::{AppError, AppResult};
use crate::store::model::Profile;

// Walk jumpHostId from the target back to a root, detecting cycles and missing
// bastions. Returned chain is ordered root-first, target-last.
pub fn resolve_jump_chain(profiles: &[Profile], target_id: &str) -> AppResult<Vec<Profile>> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = target_id.to_string();
    loop {
        if !seen.insert(current.clone()) {
            return Err(AppError::Ssh(format!("jump host cycle at profile {current}")));
        }
        let profile = profiles
            .iter()
            .find(|p| p.id == current)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("jump host profile {current}")))?;
        let next = profile.jump_host_id.clone();
        chain.push(profile);
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
    use crate::store::model::{AuthMethod, IconRef};

    fn profile(id: &str, jump: Option<&str>) -> Profile {
        Profile {
            id: id.into(),
            name: id.into(),
            group_id: None,
            host: format!("{id}.example"),
            port: 22,
            username: "me".into(),
            auth_method: AuthMethod::Agent,
            key_path: None,
            secret_id: None,
            icon: IconRef::default(),
            order: 0,
            jump_host_id: jump.map(|s| s.to_string()),
            tunnels: vec![],
            appearance: None,
        }
    }

    #[test]
    fn no_jump_is_single_element() {
        let ps = vec![profile("t", None)];
        let chain = resolve_jump_chain(&ps, "t").unwrap();
        assert_eq!(chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["t"]);
    }

    #[test]
    fn two_hops_are_root_first() {
        let ps = vec![profile("t", Some("b")), profile("b", None)];
        let chain = resolve_jump_chain(&ps, "t").unwrap();
        assert_eq!(chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", "t"]);
    }

    #[test]
    fn three_hops_chain_in_order() {
        let ps = vec![profile("t", Some("b")), profile("b", Some("a")), profile("a", None)];
        let chain = resolve_jump_chain(&ps, "t").unwrap();
        assert_eq!(chain.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "b", "t"]);
    }

    #[test]
    fn cycle_is_rejected() {
        let ps = vec![profile("t", Some("b")), profile("b", Some("t"))];
        assert!(matches!(resolve_jump_chain(&ps, "t"), Err(AppError::Ssh(_))));
    }

    #[test]
    fn missing_bastion_is_not_found() {
        let ps = vec![profile("t", Some("ghost"))];
        assert!(matches!(resolve_jump_chain(&ps, "t"), Err(AppError::NotFound(_))));
    }
}
