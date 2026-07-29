//! Stamp the build with the commit it came from.
//!
//! The crate version alone cannot answer "which build am I looking at", because
//! it does not move between commits. A short SHA does, and a `+` marks a tree
//! with uncommitted changes, which is the usual state while testing.
//!
//! Failing to find git is not an error: a source tarball has no repository and
//! must still build. It just reports the version on its own.

use std::process::Command;

fn main() {
    let stamp = match describe() {
        Some(id) => format!("{} {id}", env!("CARGO_PKG_VERSION")),
        None => env!("CARGO_PKG_VERSION").to_string(),
    };
    println!("cargo:rustc-env=CLIPPY_BUILD={stamp}");
    // Without this the stamp is baked once and never refreshed, which is worse
    // than no stamp: it would name the wrong commit with confidence.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}

fn describe() -> Option<String> {
    let sha = run(&["rev-parse", "--short=7", "HEAD"])?;
    let dirty = run(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    Some(format!("{sha}{}", if dirty { "+" } else { "" }))
}

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
