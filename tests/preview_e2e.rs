//! Live-pacman regression tests. Require Arch (bash is always installed).
//! Run with: cargo test --test preview_e2e -- --ignored
//!
//! Unlike the unit tests (fixed fixtures), these run the real preview
//! function against the real pacman, covering arg passing, stream routing,
//! and output merging. They would have caught the stdout/stderr mixup.

use alpm_cleaner::resolve::run_pacman_preview;

fn bash() -> Vec<String> {
    vec!["bash".to_string()]
}

#[test]
#[ignore]
fn bash_noncascade_reports_breakages() {
    let p = run_pacman_preview(&bash(), false);
    assert!(p.removed.is_empty(), "failed transaction removes nothing");
    assert!(!p.breakages.is_empty(), "dependents reported as breakages");
    assert!(
        p.breakages.iter().any(|(_, t)| t == "bash"),
        "breakages name the requested package"
    );
    assert_eq!(p.error, None, "no raw error text when breakages explain it");
    assert!(!p.hold_pkg);
}

#[test]
#[ignore]
fn bash_cascade_hits_holdpkg() {
    // `bash` pulls in HoldPkg'd packages under `-c`; pacman prints no list.
    let p = run_pacman_preview(&bash(), true);
    assert!(p.hold_pkg);
    assert!(p.removed.is_empty());
}
