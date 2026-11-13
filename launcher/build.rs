use std::path::PathBuf;

fn main() {
    // The SDK's linker script, by absolute path from this crate's location, so the build works from any
    // directory. `PSOXIDE_LINK_ELF=1` (psoxide-pgo apply) keeps the ELF so the driver can read its DWARF;
    // cargo passes build-script link arguments after rustflags, so the flat output format is added here
    // and not in the config.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ld = manifest.join("../games/PSoXide-sdk/sdk/psoxide.ld");
    let ld = ld.canonicalize().unwrap_or(ld);
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ELF");
    if std::env::var_os("PSOXIDE_LINK_ELF").is_none() {
        println!("cargo:rustc-link-arg=--oformat=binary");
    }
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ORDER");
    if let Some(order) = std::env::var_os("PSOXIDE_LINK_ORDER") {
        println!("cargo:rustc-link-arg=--symbol-ordering-file={}", order.to_string_lossy());
    }
    // Neither is a file cargo can track: without these a version or blob
    // change leaves a stale launcher behind.
    println!("cargo:rerun-if-env-changed=DISC_VERSION");
    println!("cargo:rerun-if-env-changed=DISC_TITLE");
    println!("cargo:rerun-if-env-changed=LOADER_BLOB");
    // The generated song table the menu music is built around.
    println!("cargo:rerun-if-env-changed=XA_SONGS_RS");
    if let Ok(path) = std::env::var("XA_SONGS_RS") {
        println!("cargo:rerun-if-changed={path}");
    }
}
