use std::path::PathBuf;

fn main() {
    // The SDK's linker script, by absolute path from this crate's location. `PSOXIDE_LINK_ELF=1`
    // (psoxide-pgo apply) keeps the ELF for the driver; cargo passes build-script link arguments after
    // rustflags, so the flat output format is added here.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ld = manifest.join("../../games/PSoXide-sdk/sdk/psoxide.ld");
    let ld = ld.canonicalize().unwrap_or(ld);
    println!("cargo:rustc-link-arg=-T{}", ld.display());
    println!("cargo:rerun-if-changed={}", ld.display());
    println!("cargo:rerun-if-env-changed=PSOXIDE_LINK_ELF");
    if std::env::var_os("PSOXIDE_LINK_ELF").is_none() {
        println!("cargo:rustc-link-arg=--oformat=binary");
    }
}
