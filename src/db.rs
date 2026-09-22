use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;

/// A single installed package from the local ALPM db.
#[derive(Debug, Clone)]
pub struct Pkg {
    pub name: String,
    pub version: String,
    pub desc: String,
    /// Installed size in bytes.
    pub size: u64,
    /// true = installed as dependency, false = explicitly installed.
    pub as_dep: bool,
    /// true = foreign (AUR / manual), false = from sync repos.
    pub foreign: bool,
    pub url: String,
    pub groups: Vec<String>,
    pub licenses: Vec<String>,
    /// Raw dependency specs (version constraints kept for display).
    pub depends: Vec<String>,
    /// Raw optional deps (`name: reason`).
    pub optdepends: Vec<String>,
    /// Raw provided names.
    pub provides: Vec<String>,
    /// Install time as unix epoch.
    pub install_date: Option<i64>,
}

/// Normalize a dep/provide name: strip version constraints
/// (`libreadline.so=8-64` -> `libreadline.so`).
pub fn dep_key(s: &str) -> String {
    s.find(['=', '<', '>'])
        .map(|i| &s[..i])
        .unwrap_or(s)
        .trim()
        .to_string()
}

/// Names of installed packages directly depending on `target`,
/// resolving through `provides` (e.g. dependents on `sh` find bash).
/// Sorted.
pub fn required_by<'a>(pkgs: &'a [Pkg], target: &Pkg) -> Vec<&'a str> {
    let mut keys: HashSet<String> = HashSet::new();
    keys.insert(target.name.clone());
    for p in &target.provides {
        keys.insert(dep_key(p));
    }
    let mut v: Vec<&str> = pkgs
        .iter()
        .filter(|q| q.name != target.name && q.depends.iter().any(|d| keys.contains(&dep_key(d))))
        .map(|q| q.name.as_str())
        .collect();
    v.sort_unstable();
    v
}

fn parse_desc(path: &std::path::Path) -> Option<Pkg> {
    let text = fs::read_to_string(path).ok()?;
    let mut name = String::new();
    let mut version = String::new();
    let mut desc = String::new();
    let mut size: u64 = 0;
    let mut as_dep = false;
    let mut url = String::new();
    let mut groups = Vec::new();
    let mut licenses = Vec::new();
    let mut depends = Vec::new();
    let mut optdepends = Vec::new();
    let mut provides = Vec::new();
    let mut install_date = None;

    let mut cur: Option<&str> = None;
    for line in text.lines() {
        if line.starts_with('%') && line.ends_with('%') {
            cur = match line {
                "%NAME%" => Some("name"),
                "%VERSION%" => Some("version"),
                "%DESC%" => Some("desc"),
                "%URL%" => Some("url"),
                "%SIZE%" => Some("size"),
                "%REASON%" => Some("reason"),
                "%GROUPS%" => Some("groups"),
                "%LICENSE%" => Some("licenses"),
                "%DEPENDS%" => Some("depends"),
                "%OPTDEPENDS%" => Some("optdepends"),
                "%PROVIDES%" => Some("provides"),
                "%INSTALLDATE%" => Some("installdate"),
                _ => None,
            };
            continue;
        }
        if line.is_empty() {
            continue;
        }
        match cur {
            Some("name") if name.is_empty() => name = line.trim().to_string(),
            Some("version") if version.is_empty() => {
                version = line.trim().to_string();
            }
            Some("desc") if desc.is_empty() => desc = line.trim().to_string(),
            Some("url") => {
                url = line.trim().to_string();
                cur = None; // single-value fields: stop consuming
            }
            Some("size") => {
                size = line.trim().parse().unwrap_or(0);
                cur = None;
            }
            Some("reason") => {
                as_dep = line.trim() == "1";
                cur = None;
            }
            Some("installdate") => {
                install_date = line.trim().parse().ok();
                cur = None;
            }
            Some("groups") => groups.push(line.trim().to_string()),
            Some("licenses") => licenses.push(line.trim().to_string()),
            Some("depends") => depends.push(line.trim().to_string()),
            Some("optdepends") => optdepends.push(line.trim().to_string()),
            Some("provides") => provides.push(line.trim().to_string()),
            _ => {}
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(Pkg {
        name,
        version,
        desc,
        size,
        as_dep,
        foreign: false,
        url,
        groups,
        licenses,
        depends,
        optdepends,
        provides,
        install_date,
    })
}

fn foreign_set() -> HashSet<String> {
    let out = Command::new("pacman").arg("-Qm").output();
    let mut set = HashSet::new();
    if let Ok(out) = out {
        if out.status.success() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let n = line.split_whitespace().next().unwrap_or("");
                if !n.is_empty() {
                    set.insert(n.to_string());
                }
            }
        }
    }
    set
}

/// Load all installed packages. Reads `/var/lib/pacman/local/*/desc`
/// (fast, no per-package subprocess) plus one `pacman -Qm` for AUR marking.
pub fn load_db() -> Result<(Vec<Pkg>, HashMap<String, usize>)> {
    let db_path = std::env::var("ALPM_CLEANER_DB")
        .unwrap_or_else(|_| "/var/lib/pacman/local".to_string());
    let entries = fs::read_dir(&db_path)
        .with_context(|| format!("cannot read pacman db at {db_path}"))?;

    let mut pkgs = Vec::new();
    for e in entries.flatten() {
        let desc = e.path().join("desc");
        if desc.is_file() {
            if let Some(p) = parse_desc(&desc) {
                pkgs.push(p);
            }
        }
    }
    if pkgs.is_empty() {
        anyhow::bail!("no packages found in {db_path}");
    }

    let foreign = foreign_set();
    for p in &mut pkgs {
        p.foreign = foreign.contains(&p.name);
    }

    pkgs.sort_by(|a, b| a.name.cmp(&b.name));
    let idx: HashMap<String, usize> = pkgs
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.clone(), i))
        .collect();
    Ok((pkgs, idx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(name: &str, depends: &[&str], provides: &[&str]) -> Pkg {
        Pkg {
            name: name.into(),
            version: "1".into(),
            desc: String::new(),
            size: 0,
            as_dep: false,
            foreign: false,
            url: String::new(),
            groups: vec![],
            licenses: vec![],
            depends: depends.iter().map(|s| s.to_string()).collect(),
            optdepends: vec![],
            provides: provides.iter().map(|s| s.to_string()).collect(),
            install_date: None,
        }
    }

    #[test]
    fn dep_key_strips_constraints() {
        assert_eq!(dep_key("libreadline.so=8-64"), "libreadline.so");
        assert_eq!(dep_key("foo>=1.0"), "foo");
        assert_eq!(dep_key("plain"), "plain");
    }

    #[test]
    fn required_by_resolves_through_provides() {
        let pkgs = vec![
            pkg("bash", &[], &["sh"]),
            pkg("user", &["sh"], &[]),
            pkg("other", &[], &[]),
        ];
        let bash = &pkgs[0];
        let r = required_by(&pkgs, bash);
        assert_eq!(r, vec!["user"]);
    }
}
