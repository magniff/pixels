//! Stamp the binary with the commit it was built from.
//!
//! The About panel prints it. A screenshot of a bug is worth a great deal more
//! when it says which build took it.

fn main() {
    let rev = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|rev| !rev.is_empty())
        .unwrap_or_else(|| "unknown".to_string());

    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|out| !out.stdout.is_empty());

    println!(
        "cargo:rustc-env=GIT_REV={rev}{}",
        if dirty { "+" } else { "" }
    );
    // Rebuilt when the commit changes, and not otherwise.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}
