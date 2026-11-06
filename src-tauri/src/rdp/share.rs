use std::path::{Path, PathBuf};

pub struct Share<'a> {
    pub root: &'a Path,
    pub name: &'a str,
}

#[derive(Debug)]
pub(crate) struct Denied;

pub(crate) fn valid_name(name: &str) -> bool {
    (1..=7).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

// Windows separates with `\`, xrdp mixes `\` and `/`.
pub(crate) fn parts(server_path: &str) -> Result<Vec<&str>, Denied> {
    let parts: Vec<&str> = server_path.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
    if parts.iter().all(|part| allowed(part)) {
        Ok(parts)
    } else {
        Err(Denied)
    }
}

fn allowed(part: &str) -> bool {
    // Windows drops trailing dots and spaces, so `.`, `..`, `...` and `.. ` all trim to nothing.
    let trimmed = part.trim_end_matches(['.', ' ']);
    let stem = trimmed.split('.').next().unwrap_or_default().trim_end_matches(' ');
    !trimmed.is_empty()
        && !part.chars().any(|c| c < ' ' || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && !reserved(stem)
}

fn reserved(stem: &str) -> bool {
    let stem = stem.to_ascii_uppercase();
    let numbered = |device: &str| {
        stem.strip_prefix(device).is_some_and(|n| {
            matches!(n, "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        })
    };
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || numbered("COM")
        || numbered("LPT")
}

// `root` is canonical. A `new` path is one to create or rename to: its folder has to exist, the
// last part may not.
pub(crate) fn resolve(root: &Path, server_path: &str, new: bool) -> Result<PathBuf, Denied> {
    let parts = parts(server_path)?;
    let resolved = if new {
        let (name, folders) = parts.split_last().ok_or(Denied)?;
        let mut parent = root.to_path_buf();
        parent.extend(folders);
        let parent = std::fs::canonicalize(parent).map_err(|_| Denied)?;
        if !parent.is_dir() {
            return Err(Denied);
        }
        parent.join(name)
    } else {
        let mut path = root.to_path_buf();
        path.extend(&parts);
        std::fs::canonicalize(path).map_err(|_| Denied)?
    };
    if resolved.starts_with(root) {
        Ok(resolved)
    } else {
        Err(Denied)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    // A share with a file, a folder with a file in it and a folder `double` holding `sep`, next
    // to a folder `share2` whose name starts like the share's.
    fn share() -> (TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("share");
        let outside = dir.path().join("share2");
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join("double")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("readme.txt"), "hello").unwrap();
        fs::write(root.join("sub").join("inner.txt"), "inner").unwrap();
        fs::write(root.join("double").join("sep"), "sep").unwrap();
        fs::write(outside.join("secret.txt"), "secret").unwrap();
        let root = fs::canonicalize(root).unwrap();
        let outside = fs::canonicalize(outside).unwrap();
        (dir, root, outside)
    }

    #[test]
    fn share_names_have_one_to_seven_letters_digits_dashes_or_underscores() {
        for name in ["wisp", "a", "Doc_1-2", "1234567"] {
            assert!(valid_name(name), "{name:?} was refused");
        }
        for name in ["", "toolong8", "wi sp", "wïsp", "a/b", "a\\b", "a.b", "a:"] {
            assert!(!valid_name(name), "{name:?} was taken");
        }
    }

    #[test]
    fn paths_that_leave_or_misname_the_share_are_refused() {
        let (_dir, root, _) = share();
        let refused = [
            "..",
            "\\..",
            "a\\..\\..\\x",
            "sub/../x",
            "...",
            ".. ",
            ".",
            "\\sub\\.",
            "C:\\Windows",
            "C:foo",
            "\\\\?\\C:\\x",
            "readme.txt:ads",
            "readme.txt::$DATA",
            "CON",
            "con.txt",
            "NUL.",
            "COM1 ",
            "lpt9.log",
            "COM¹",
            "lpt³.txt",
            "CONIN$",
            "conout$.x",
            "\\sub\\aux",
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "a\u{1}b",
            "a\u{0}b",
            "tab\there",
        ];
        for path in refused {
            assert!(resolve(&root, path, false).is_err(), "{path:?} was taken");
            assert!(resolve(&root, path, true).is_err(), "{path:?} was taken as a new name");
        }
    }

    #[test]
    fn names_that_only_resemble_reserved_ones_are_taken() {
        let (_dir, root, _) = share();
        for name in ["CONSOLE", "COM10", "LPT", "nul2.txt", "auxiliary", ".hidden", "a.", "x y"] {
            assert_eq!(resolve(&root, name, true).unwrap(), root.join(name), "{name:?}");
        }
    }

    #[test]
    fn paths_inside_the_share_resolve_under_its_root() {
        let (_dir, root, _) = share();
        let inside = [
            ("", root.clone()),
            ("\\", root.clone()),
            ("/", root.clone()),
            ("\\readme.txt", root.join("readme.txt")),
            ("/sub/inner.txt", root.join("sub").join("inner.txt")),
            ("\\sub\\", root.join("sub")),
            ("\\\\double\\\\sep", root.join("double").join("sep")),
            ("sub/\\inner.txt", root.join("sub").join("inner.txt")),
        ];
        for (path, expected) in inside {
            assert_eq!(resolve(&root, path, false).unwrap(), expected, "{path:?}");
        }
    }

    #[test]
    fn a_missing_path_does_not_resolve_unless_it_is_new() {
        let (_dir, root, _) = share();
        assert!(resolve(&root, "\\new.txt", false).is_err());
        assert_eq!(resolve(&root, "\\new.txt", true).unwrap(), root.join("new.txt"));
        assert_eq!(resolve(&root, "/sub/new.txt", true).unwrap(), root.join("sub").join("new.txt"));
        assert_eq!(resolve(&root, "\\readme.txt", true).unwrap(), root.join("readme.txt"));
    }

    #[test]
    fn a_new_name_in_a_missing_folder_is_refused() {
        let (_dir, root, _) = share();
        assert!(resolve(&root, "\\missing\\new.txt", true).is_err());
        assert!(resolve(&root, "\\readme.txt\\new.txt", true).is_err());
    }

    #[test]
    fn the_share_root_is_no_new_name() {
        let (_dir, root, _) = share();
        for path in ["", "\\", "/"] {
            assert!(resolve(&root, path, true).is_err(), "{path:?}");
        }
    }

    fn refuses_the_way_out(root: &Path, link: &str) {
        let existing = [link.to_owned(), format!("{link}\\secret.txt"), format!("{link}/x.txt")];
        for path in existing {
            assert!(resolve(root, &path, false).is_err(), "{path:?} was taken");
        }
        for path in [format!("{link}\\secret.txt"), format!("{link}\\new.txt")] {
            assert!(resolve(root, &path, true).is_err(), "{path:?} was taken as a new name");
        }
    }

    #[test]
    fn a_symlink_out_of_the_share_is_refused() {
        let (_dir, root, outside) = share();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("out")).unwrap();
            std::os::unix::fs::symlink(root.join("sub"), root.join("in")).unwrap();
        }
        #[cfg(windows)]
        {
            // Without developer mode Windows creates no symbolic link.
            if let Err(e) = std::os::windows::fs::symlink_dir(&outside, root.join("out")) {
                println!("no symbolic link without developer mode ({e}), test skipped");
                return;
            }
            std::os::windows::fs::symlink_dir(root.join("sub"), root.join("in")).unwrap();
        }
        refuses_the_way_out(&root, "out");
        let inner = root.join("sub").join("inner.txt");
        assert_eq!(resolve(&root, "in\\inner.txt", false).unwrap(), inner);
        assert_eq!(resolve(&root, "in\\new.txt", true).unwrap(), root.join("sub").join("new.txt"));
    }

    #[cfg(windows)]
    fn junction(target: &Path, at: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(at)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "mklink /J failed: {made:?}");
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_out_of_the_share_is_refused() {
        let (dir, root, _) = share();
        junction(&dir.path().join("share2"), &dir.path().join("share").join("out"));
        junction(&dir.path().join("share").join("sub"), &dir.path().join("share").join("in"));
        refuses_the_way_out(&root, "out");
        let inner = root.join("sub").join("inner.txt");
        assert_eq!(resolve(&root, "in\\inner.txt", false).unwrap(), inner);
    }
}
