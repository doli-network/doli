use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let hash = hash.trim();
    let version = env!("CARGO_PKG_VERSION");
    if hash.is_empty() {
        println!("cargo:rustc-env=DOLI_VERSION_STRING={version}");
    } else {
        println!("cargo:rustc-env=DOLI_VERSION_STRING={version} ({hash})");
    }
}
