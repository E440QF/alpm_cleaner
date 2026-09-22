use std::process::Command;

/// How to gain root for the final `pacman -R...` run.
#[derive(Debug, Clone)]
pub enum Privilege {
    /// Already root / run directly.
    None,
    /// Prefix command, e.g. `sudo`, `doas`, `run0`.
    Prefix(String),
}

impl Privilege {
    pub fn prefix_args(&self) -> Vec<String> {
        match self {
            Privilege::None => vec![],
            Privilege::Prefix(p) => vec![p.clone()],
        }
    }

    pub fn display(&self) -> &str {
        match self {
            Privilege::None => "(root)",
            Privilege::Prefix(p) => p.as_str(),
        }
    }
}

fn bin_exists(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn is_root() -> bool {
    // Cheap euid check without pulling in libc: `id -u`.
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}

/// Order:
/// 1. already root -> none
/// 2. `$SUDO` / `$ALPM_CLEANER_PRIV` override when set (even to empty = none)
/// 3. `sudo` if installed, else `doas`, else `run0`, else none (apply will fail with a message).
pub fn detect() -> Privilege {
    if is_root() {
        return Privilege::None;
    }
    for var in ["ALPM_CLEANER_PRIV", "SUDO"] {
        if let Ok(v) = std::env::var(var) {
            let v = v.trim().to_string();
            if v.is_empty() {
                return Privilege::None;
            }
            return Privilege::Prefix(v);
        }
    }
    for cand in ["sudo", "doas", "run0"] {
        if bin_exists(cand) {
            return Privilege::Prefix(cand.to_string());
        }
    }
    Privilege::None
}

/// Build the apply command argv (without privilege prefix).
pub fn pacman_argv(cascade: bool, targets: &[String]) -> Vec<String> {
    let mut v = vec![
        "pacman".to_string(),
        "-Rns".to_string(),
    ];
    if cascade {
        v.push("-c".to_string());
    }
    v.extend(targets.iter().cloned());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_shapes() {
        assert_eq!(
            pacman_argv(false, &["a".into()]),
            vec!["pacman", "-Rns", "a"]
        );
        // -Rns -c == -Rnsc
        assert_eq!(
            pacman_argv(true, &["a".into(), "b".into()]),
            vec!["pacman", "-Rns", "-c", "a", "b"]
        );
    }
}
