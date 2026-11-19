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
    external_art();
}

/// The programs in the cover table's fixed order (`ALL` in `cover.rs` lists the
/// same ids in the same order, and a const assertion there checks it), with
/// where each one's art comes from: `None` is a `launcher-art` directory
/// (`LAUNCHER_ART_DIR`, the baked blobs of `menu-art ext`), `Some(dir)` a
/// directory of this crate (none now). Cortex appears twice, for its Public and Private
/// entries.
const PROGRAMS: &[(&str, Option<&str>)] = &[
    ("arcade", None),
    ("celeste", None),
    ("nitro", None),
    ("voxide", None),
    ("quake", None),
    ("cortex", None),
    ("cortex", None),
    ("psxcel", None),
    ("wipeout", None),
    ("hollow", None),
    ("cstrike", None),
    ("halflife", None),
    ("oot", None),
    ("citadel", None),
    ("x360", None),
    ("qdup", None),
];

/// Writes `art.rs`, which `cover.rs` includes: `IDS` and, per program, `Some`
/// blobs when all four files are there and `None` otherwise (a build with a
/// directory missing keeps that program's plain cover). It is plain data, with
/// no cfg flag, so the launcher's code and symbol names do not depend on which
/// art was supplied. The files are named by absolute path only inside the
/// generated source (`include_bytes!` does not put the path in the binary).
fn external_art() {
    println!("cargo:rerun-if-env-changed=LAUNCHER_ART_DIR");
    let ext = std::env::var_os("LAUNCHER_ART_DIR").map(|d| {
        let d = PathBuf::from(d);
        d.canonicalize().unwrap_or(d)
    });
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mut src = String::new();
    src.push_str(&format!("const IDS: [&str; {}] = [", PROGRAMS.len()));
    for (id, _) in PROGRAMS {
        src.push_str(&format!("{id:?}, "));
    }
    src.push_str("];\n");
    src.push_str(&format!("const ART: [Option<Blobs>; {}] = [\n", PROGRAMS.len()));
    for (id, home) in PROGRAMS {
        let dir = match home {
            Some(d) => Some(manifest.join(d)),
            None => ext.clone(),
        };
        let files = dir.map(|d| ["art_l_", "art_m_", "art_s_", "back_"].map(|p| d.join(format!("{p}{id}.art"))));
        match files {
            Some(f) => {
                for p in &f {
                    println!("cargo:rerun-if-changed={}", p.display());
                }
                if f.iter().all(|p| p.is_file()) {
                    let q = |p: &PathBuf| format!("include_bytes!({:?})", p.display().to_string());
                    src.push_str(&format!(
                        "    Some(Blobs {{ l: {}, m: {}, s: {}, back: {} }}),\n",
                        q(&f[0]),
                        q(&f[1]),
                        q(&f[2]),
                        q(&f[3])
                    ));
                    continue;
                }
            }
            None => {}
        }
        src.push_str("    None,\n");
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("art.rs");
    std::fs::write(out, src).expect("write art.rs");
}
