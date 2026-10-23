//! Links the blob at the address `chainload-layout` names.
//!
//! The linker script is a template with the base and the size left open, so the
//! launcher, `mkdisc` and the link cannot disagree about where the blob runs.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=loader.ld");
    let template = fs::read_to_string("loader.ld").expect("read loader.ld");
    let script = template
        .replace(
            "@LOADER_BASE@",
            &format!("{:#010X}", chainload_layout::LOADER_BASE),
        )
        .replace(
            "@LOADER_BUDGET@",
            &chainload_layout::LOADER_BUDGET.to_string(),
        );
    assert!(!script.contains('@'), "loader.ld has an unfilled placeholder");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("loader.ld");
    fs::write(&out, script).expect("write the generated linker script");
    println!("cargo:rustc-link-arg=-T{}", out.display());
}
