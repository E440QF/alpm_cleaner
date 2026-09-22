use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

/// Exact removal preview from `pacman -Rs[c] --print`.
///
/// The old graph approximation over-pulled orphans (every pre-existing
/// orphan in the db, not just the target's subtree). Querying pacman
/// directly is authoritative, needs no root, and takes ~0.3s, so it runs
/// in a background thread and never blocks the TUI.
///
/// Note: `-n` (nosave) only affects backup-file handling, not the package
/// set, so `-Rs[c] --print` previews `-Rns[c]` exactly.
#[derive(Debug, Clone, Default)]
pub struct Preview {
    /// Every package pacman would remove (explicit + pulled), sorted.
    pub removed: Vec<String>,
    /// `pacman` error output that isn't a breakage/missing/HoldPkg line.
    pub error: Option<String>,
    /// Checked names that are not installed (`target not found`).
    pub missing: Vec<String>,
    /// `(dependent, requested)` from `removing X breaks dependency ... required by Y`.
    /// Only populated in non-cascade mode (transaction refused).
    pub breakages: Vec<(String, String)>,
    /// A HoldPkg (e.g. pacman) is in the target list; pacman prints no list.
    pub hold_pkg: bool,
}

impl Preview {
    pub fn selected(&self, checked: &HashSet<String>) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .removed
            .iter()
            .filter(|n| checked.contains(*n))
            .map(|s| s.as_str())
            .collect();
        v.sort_unstable();
        v
    }

    pub fn pulled(&self, checked: &HashSet<String>) -> Vec<&str> {        let mut v: Vec<&str> = self
            .removed
            .iter()
            .filter(|n| !checked.contains(*n))
            .map(|s| s.as_str())
            .collect();
        v.sort_unstable();
        v
    }
}

/// Parse one `:: removing X breaks dependency '...' required by Y` line.
/// These informational lines go to **stdout**, same stream as the
/// `--print-format %n` package list.
fn parse_breakage(l: &str) -> Option<(String, String)> {
    let rest = l.strip_prefix(":: removing ")?;
    let (target, tail) = rest.split_once(" breaks dependency ")?;
    let (_, dependent) = tail.split_once(" required by ")?;
    let dep = dependent.split_whitespace().next().filter(|s| !s.is_empty())?;
    Some((dep.to_string(), target.trim().to_string()))
}

/// Parse stdout of `pacman --print --print-format %n`: bare names on
/// success; `:: removing …` breakage lines and the HoldPkg prompt on
/// failure. Returns (removed, breakages, hold_pkg).
fn parse_stdout(stdout: &str) -> (Vec<String>, Vec<(String, String)>, bool) {
    let mut removed = Vec::new();
    let mut breakages = Vec::new();
    let mut hold_pkg = false;
    for line in stdout.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(b) = parse_breakage(l) {
            breakages.push(b);
        } else if l.contains("HoldPkg was found in target list") {
            hold_pkg = true;
        } else if l.starts_with("::") || l.contains(char::is_whitespace) {
            // Other pacman chatter: never a package name, ignore.
        } else {
            removed.push(l.to_string());
        }
    }
    removed.sort();
    removed.dedup();
    breakages.sort();
    breakages.dedup();
    (removed, breakages, hold_pkg)
}

/// (breakages, missing, hold_pkg, other_lines).
type StderrParse = (
    Vec<(String, String)>,
    Vec<String>,
    bool,
    Vec<String>,
);

/// Parse stderr: `error: target not found`, the transaction summary, and
/// (defensively) breakage/HoldPkg lines should they ever land here.
/// Returns (breakages, missing, hold_pkg, other_lines).
fn parse_stderr(stderr: &str) -> StderrParse {
    let mut breakages = Vec::new();
    let mut missing = Vec::new();
    let mut hold_pkg = false;
    let mut other: Vec<String> = Vec::new();

    for line in stderr.lines() {
        let l = line.trim();
        if let Some(b) = parse_breakage(l) {
            breakages.push(b);
        } else if let Some(name) = l.strip_prefix("error: target not found: ") {
            let n = name.split_whitespace().next().unwrap_or("").to_string();
            if !n.is_empty() {
                missing.push(n);
            }
        } else if l.contains("HoldPkg was found in target list") {
            hold_pkg = true;
        } else if !l.is_empty() {
            other.push(l.to_string());
        }
    }

    breakages.sort();
    breakages.dedup();
    missing.sort();
    missing.dedup();
    (breakages, missing, hold_pkg, other)
}

/// Run pacman synchronously. Stdin is nulled so HoldPkg-style prompts get
/// EOF instead of hanging.
pub fn run_pacman_preview(targets: &[String], cascade: bool) -> Preview {
    let mut args = vec!["-Rs".to_string()];
    if cascade {
        args.push("-c".to_string());
    }
    args.push("--print".to_string());
    args.push("--noconfirm".to_string());
    args.push("--print-format".to_string());
    args.push("%n".to_string());
    args.extend(targets.iter().cloned());

    let out = Command::new("pacman")
        .args(&args)
        .stdin(Stdio::null())
        .output();

    match out {
        Err(e) => Preview {
            error: Some(format!("failed to run pacman: {e}")),
            ..Default::default()
        },
        Ok(out) => {
            let (removed, mut breakages, out_hold) =
                parse_stdout(&String::from_utf8_lossy(&out.stdout));
            let (err_breakages, missing, err_hold, mut other) =
                parse_stderr(&String::from_utf8_lossy(&out.stderr));
            breakages.extend(err_breakages);
            breakages.sort();
            breakages.dedup();
            // The "failed to prepare transaction" summary is noise when the
            // breakage lines explain the failure.
            if !breakages.is_empty() {
                other.retain(|l| !l.starts_with("error: failed to prepare transaction"));
            }
            let error = if other.is_empty() {
                None
            } else {
                Some(other.join("\n"))
            };
            Preview {
                removed,
                error,
                missing,
                breakages: if cascade { Vec::new() } else { breakages },
                hold_pkg: out_hold || err_hold,
            }
        }
    }
}

struct Request {
    generation: u64,
    targets: Vec<String>,
    cascade: bool,
}

/// Background worker: keeps only the latest queued request so rapid
/// toggling never piles up (~0.3-0.9s per pacman run).
pub struct PreviewEngine {
    tx: Sender<Request>,
    rx: Receiver<(u64, Preview)>,
    generation: u64,
    _handle: JoinHandle<()>,
}

impl PreviewEngine {
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<Request>();
        let (res_tx, res_rx) = mpsc::channel::<(u64, Preview)>();
        let handle = thread::spawn(move || {
            while let Ok(first) = req_rx.recv() {
                // Drain: only the newest request matters.
                let mut cur = first;
                while let Ok(newer) = req_rx.try_recv() {
                    cur = newer;
                }
                let preview = run_pacman_preview(&cur.targets, cur.cascade);
                if res_tx.send((cur.generation, preview)).is_err() {
                    break;
                }
            }
        });
        Self {
            tx: req_tx,
            rx: res_rx,
            generation: 0,
            _handle: handle,
        }
    }

    /// Ask for a fresh preview; returns the generation to match results with.
    pub fn request(&mut self, targets: Vec<String>, cascade: bool) -> u64 {
        self.generation += 1;
        let gen = self.generation;
        let _ = self.tx.send(Request {
            generation: gen,
            targets,
            cascade,
        });
        gen
    }

    pub fn poll(&self, want: u64) -> Option<Preview> {
        let mut latest = None;
        while let Ok((gen, p)) = self.rx.try_recv() {
            if gen == want {
                latest = Some(p);
            }
            // Stale generations are dropped.
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_parses_bare_names() {
        let (removed, brk, hold) = parse_stdout("zed\napple\nmango\n");
        assert_eq!(removed, vec!["apple", "mango", "zed"]);
        assert!(brk.is_empty());
        assert!(!hold);
    }

    #[test]
    fn stdout_parses_breakages_not_packages() {
        // Regression: these lines go to stdout, and were previously shown
        // as removal entries while the error line filled the panel.
        let out = ":: removing bash breaks dependency 'sh' required by shell-user\n\
                   :: removing bash breaks dependency 'bash' required by alsa-utils\n";
        let (removed, brk, hold) = parse_stdout(out);
        assert!(removed.is_empty(), "breakage lines are not packages");
        assert!(brk.contains(&("shell-user".to_string(), "bash".to_string())));
        assert!(brk.contains(&("alsa-utils".to_string(), "bash".to_string())));
        assert!(!hold);
    }

    #[test]
    fn stdout_detects_holdpkg() {
        let (removed, _, hold) =
            parse_stdout(":: HoldPkg was found in target list. Do you want to continue? [y/N]\n");
        assert!(hold);
        assert!(removed.is_empty());
    }

    #[test]
    fn stderr_parses_breakages_and_missing() {
        let err = "error: failed to prepare transaction (could not satisfy dependencies)\n\
                   :: removing bash breaks dependency 'sh' required by shell-user\n\
                   :: removing bash breaks dependency 'bash' required by alsa-utils\n\
                   error: target not found: nosuchpkg123\n";
        let (brk, missing, hold, other) = parse_stderr(err);
        assert!(brk.contains(&("shell-user".to_string(), "bash".to_string())));
        assert!(brk.contains(&("alsa-utils".to_string(), "bash".to_string())));
        assert_eq!(missing, vec!["nosuchpkg123".to_string()]);
        assert!(!hold);
        assert!(other.contains(
            &"error: failed to prepare transaction (could not satisfy dependencies)".to_string()
        ));
    }

    #[test]
    fn stderr_detects_holdpkg() {
        let (_, _, hold, _) =
            parse_stderr(":: HoldPkg was found in target list. Do you want to continue? [y/N]");
        assert!(hold);
    }

    #[test]
    fn stderr_keeps_real_errors() {
        let (_, _, _, other) = parse_stderr("error: failed to lock database: File exists\n");
        assert!(other.join("\n").contains("lock"));
    }

    #[test]
    fn split_selected_vs_pulled() {
        let p = Preview {
            removed: vec!["main".into(), "dep-a".into(), "dep-b".into()],
            ..Default::default()
        };
        let checked: HashSet<String> = ["main".into()].into_iter().collect();
        assert_eq!(p.selected(&checked), vec!["main"]);
        assert_eq!(p.pulled(&checked), vec!["dep-a", "dep-b"]);
    }
}
