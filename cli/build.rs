//! Bakes the git commit into `monoplan --version`, the same shape the
//! macOS app writes into its Info.plist (`apple/MonoplanMac/build-app.sh`):
//! the 12-character short sha, "+" appended when the tree was dirty.
//! The version string does not move between dev builds; the commit does.
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_owned())
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // HEAD only changes on checkout; a commit on the current branch
    // moves the branch's ref file, so watch that too.
    for path in ["HEAD", "packed-refs", "index"] {
        if let Some(p) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    if let Some(p) = git(&["symbolic-ref", "-q", "HEAD"])
        .and_then(|head_ref| git(&["rev-parse", "--git-path", &head_ref]))
    {
        println!("cargo:rerun-if-changed={p}");
    }

    let mut sha = git(&["rev-parse", "--short=12", "HEAD"])
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    // Best effort: cargo reruns this script on commit or checkout, not on
    // every edit, so the marker can lag an uncommitted change.
    if sha != "unknown" && git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty()) {
        sha.push('+');
    }
    println!("cargo:rustc-env=MONOPLAN_BUILD_GIT_SHA={sha}");
}
