use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for key in ["TARGET", "PROFILE", "RUSTC"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    println!(
        "cargo:rustc-env=AERCAST_TARGET={}",
        env::var("TARGET").unwrap()
    );
    println!(
        "cargo:rustc-env=AERCAST_PROFILE={}",
        env::var("PROFILE").unwrap()
    );
    let compiler = Command::new(env::var_os("RUSTC").unwrap())
        .arg("--version")
        .output()
        .expect("read Rust compiler version");
    assert!(compiler.status.success(), "read Rust compiler version");
    println!(
        "cargo:rustc-env=AERCAST_RUSTC={}",
        String::from_utf8(compiler.stdout)
            .expect("Rust compiler version is UTF-8")
            .trim()
    );
}
