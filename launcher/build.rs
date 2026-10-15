fn main() {
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
