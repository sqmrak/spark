// build the gitignored seccomp_probe ELF from its C source (x86_64/aarch64).
// cc is required on a fresh checkout; a stale local copy is the only fallback.

use std::path::Path;
use std::process::Command;

fn main() {
    let src = "assets/seccomp_probe.c";
    let out = "assets/seccomp_probe";
    println!("cargo:rerun-if-changed={src}");
    println!("cargo:rerun-if-changed=build.rs");

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let ok = Command::new(&cc)
        .args(["-static", "-nostdlib", "-no-pie", "-O2", src, "-o", out])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok && !Path::new(out).exists() {
        panic!("could not build {out} from {src}; need a C compiler (x86_64 or aarch64)");
    }
}
