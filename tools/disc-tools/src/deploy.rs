//! Stage both itch.io packages from one tested public disc, then publish them.
//!
//!     disc-tools deploy-public --cue <cue> --version vX.Y --out <empty dir>
//!         [--emulator <checkout>] [--web-build <trunk dist>] [--publish]
//!
//! This is the one tool that can upload to a public page, so every guard the
//! Python original had is kept, in the same order: the cue must name one BIN
//! beside it and the disc must be exactly the public collection (menu and track
//! layout), the version must look like a release, the output directory must be
//! empty, Quake's licence goes into both packages, the emulator must be the
//! clean revision the release lock selects and pass its own component check, a
//! reused web build must carry a receipt for that revision, and no file may hit
//! itch.io's HTML size cap. Nothing is uploaded unless `--publish` is given.
//!
//! The emulator's component bootstrap belongs to the emulator repository. A pin
//! that still ships `tools/bootstrap-components.py` is checked with `python3`,
//! a newer one with the SDK's `psoxide-components` (see `components`); `trunk`
//! and `butler` are run as external programs exactly as before.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;
use serde_json::{json, Value};

use crate::args::Args;
use crate::disc::{
    le32, read_user_sectors, text_field, FLAG_HIDDEN, TOC_ENTRY_BYTES, TOC_FLAGS_AT,
    TOC_HEADER_BYTES, TOC_LBA, TOC_MAGIC, TOC_NAME_BYTES, TOC_SECTORS, USER_DATA_BYTES,
};
use crate::util::{
    dumps, git, read_json, resolve, resolve_lenient, sha256_bytes, sha256_file, Error, Result,
};

/// The menu a public pressing must show, in order: the visible table entries
/// then the launcher's own CREDITS card. PSXcel sits last before CREDITS
/// (Manny, 2026-10-05), matching the Makefile's MKDISC_ARGS order.
const PUBLIC_GAMES: [&str; 8] = [
    "PSOXIDE ARCADE",
    "CELESTE COLLECTION",
    "NITROXIDE",
    "VOXIDE",
    "QUAKE SHAREWARE",
    "CORTEX IGNITION",
    "PSXCEL",
    "CREDITS",
];
/// One title per CD-DA track, in disc order, for the browser player.
const TRACK_TITLES: [&str; 7] = [
    "KNUCKLE DUST",
    "RUSTED HAMMER",
    "CHAINSAW HEART",
    "NIGHT CRAWLER",
    "GONCHAROV",
    "CORTEX IGNITION COMBAT",
    "CORTEX IGNITION MENU",
];
const WEB_RECEIPT: &str = "emulator-build.json";
/// itch.io refuses HTML5 files at 200 MB and over.
const ITCH_FILE_LIMIT: u64 = 200_000_000;
const DOWNLOAD_TARGET: &str = "bonnie-studios/psoxide-demo-disc:psx";
const PLAYABLE_TARGET: &str = "bonnie-studios/psoxide:html5";

/// The repository this binary belongs to; the release lock, the licence and
/// the README are read from here.
fn repo_root() -> Result<PathBuf> {
    crate::util::repo_root()?.canonicalize().map_err(Into::into)
}

/// What the table says: how many entries it claims, and the menu the launcher
/// will draw. Hidden entries are not shown, and CREDITS follows the last
/// visible one, as `check_release_chainloads.parse_toc` computed it.
struct PublicToc {
    claimed: usize,
    visible: usize,
    menu: Vec<String>,
}

fn parse_public_toc(raw: &[u8]) -> Result<PublicToc> {
    ensure!(
        raw.len() == TOC_SECTORS as usize * USER_DATA_BYTES && &raw[..8] == TOC_MAGIC,
        "missing {} at LBA {TOC_LBA}",
        String::from_utf8_lossy(TOC_MAGIC)
    );
    let count = le32(raw, 8) as usize;
    let maximum = (raw.len() - TOC_HEADER_BYTES) / TOC_ENTRY_BYTES;
    ensure!(
        count != 0 && count <= maximum,
        "invalid TOC entry count {count}"
    );
    let mut names = Vec::new();
    for index in 0..count {
        let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
        let row = &raw[at..at + TOC_ENTRY_BYTES];
        let name = text_field(&row[..TOC_NAME_BYTES])?;
        if le32(row, TOC_FLAGS_AT) & FLAG_HIDDEN == 0 {
            names.push(name);
        }
    }
    let visible = names.len();
    let mut menu = names;
    if visible != 0 {
        menu.push("CREDITS".to_string());
    }
    Ok(PublicToc {
        claimed: count,
        visible,
        menu,
    })
}

/// `Path.read_text()`: UTF-8 with universal newlines, so a CRLF cue reads the
/// same as an LF one and `$` in the line patterns below still lands.
fn read_text(path: &Path) -> Result<String> {
    let text = fs::read_to_string(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

/// The BIN beside `cue`, once the disc is proven to be the public collection:
/// one adjacent BIN, the public menu, and the public track layout.
fn public_image(cue: &Path) -> Result<PathBuf> {
    let text = read_text(cue)?;
    let file_line = Regex::new(r#"(?m)^FILE "([^"]+)" BINARY$"#)?;
    let files: Vec<&str> = file_line
        .captures_iter(&text)
        .map(|c| c.get(1).map_or("", |m| m.as_str()))
        .collect();
    // `Path(name).name != name`: anything with a directory part is not adjacent.
    let adjacent =
        |name: &str| name == ".." || Path::new(name).file_name().is_some_and(|n| n == name);
    ensure!(
        files.len() == 1 && adjacent(files[0]),
        "expected one adjacent BIN file"
    );
    let image = cue.parent().unwrap_or(Path::new(".")).join(files[0]);
    let raw = read_user_sectors(&image, TOC_LBA, TOC_SECTORS)?;
    let toc = parse_public_toc(&raw)?;
    let programs = PUBLIC_GAMES.len() - 1; // every card but CREDITS
    ensure!(
        toc.menu == PUBLIC_GAMES && toc.visible == programs && toc.claimed == programs,
        "disc menu does not match the public collection"
    );
    let track_line = Regex::new(r"(?m)^\s*TRACK (\d+) (\S+)")?;
    let tracks: Vec<(String, String)> = track_line
        .captures_iter(&text)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect();
    let mut expected = vec![("01".to_string(), "MODE2/2352".to_string())];
    expected.extend((2..TRACK_TITLES.len() + 2).map(|n| (format!("{n:02}"), "AUDIO".to_string())));
    ensure!(
        tracks == expected,
        "expected the public disc's {}-track layout",
        expected.len()
    );
    Ok(image)
}

/// A release version such as `v0.34`, `v1.2.3` or `v0.34-rc.1`.
fn valid_version(version: &str) -> Result<bool> {
    Ok(Regex::new(r"\Av[0-9]+\.[0-9]+(?:\.[0-9]+)?(?:[-.][A-Za-z0-9.]+)?\z")?.is_match(version))
}

/// Every file under `directory`, as `(relative path, absolute path)` in
/// Python's `sorted(rglob("*"))` order: path components compared one by one, so
/// `a/b` sorts before `a-c`. Directory symlinks are listed (as non-files) but
/// not entered, as `rglob` does.
fn files_under(directory: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
    fn walk(directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(&path, found)?;
            } else if path.is_file() {
                found.push(path);
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    walk(directory, &mut found)?;
    found.sort();
    Ok(found
        .into_iter()
        .filter_map(|path| {
            path.strip_prefix(directory)
                .ok()
                .map(|relative| (relative.to_path_buf(), path.clone()))
        })
        .collect())
}

/// What a web build's receipt must say: which emulator source and component
/// lock it was built from, and the hash of every file in it (except the
/// receipt itself).
fn web_record(directory: &Path, revision: &str, lock_sha256: &str) -> Result<Value> {
    let mut files = serde_json::Map::new();
    for (relative, path) in files_under(directory)? {
        if path.file_name().is_some_and(|n| n == WEB_RECEIPT) {
            continue;
        }
        let key = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        files.insert(key, Value::String(sha256_file(&path)?));
    }
    Ok(json!({
        "schema": 1,
        "emulator_revision": revision,
        "components_lock_sha256": lock_sha256,
        "files": Value::Object(files),
    }))
}

/// A reused web build is only trusted if its receipt matches the locked
/// emulator and every byte now in the directory.
fn verify_web_build(directory: &Path, revision: &str, lock_sha256: &str) -> Result<()> {
    let path = directory.join(WEB_RECEIPT);
    ensure!(
        path.is_file(),
        "reused web build has no emulator source and content receipt; rebuild it"
    );
    let recorded = read_json(&path)?;
    ensure!(
        recorded == web_record(directory, revision, lock_sha256)?,
        "reused web build differs from the locked emulator or its content receipt"
    );
    Ok(())
}

/// The external programs the tool runs. They are fields so the tests can point
/// them at fake scripts; a real run uses the names the original used.
struct Tools {
    python: OsString,
    trunk: OsString,
    butler: OsString,
}

impl Tools {
    fn real() -> Tools {
        Tools {
            python: "python3".into(),
            trunk: "trunk".into(),
            butler: "butler".into(),
        }
    }
}

/// Run a program with the caller's stdio; a nonzero exit is an error.
fn run_program(command: &mut Command) -> Result<()> {
    let shown = format!("{:?}", command.get_program());
    let status = command
        .status()
        .map_err(|e| Error(format!("cannot run {shown}: {e}")))?;
    ensure!(status.success(), "{shown} failed ({status})");
    Ok(())
}

/// The emulator checkout's revision and the hash of its component lock, once it
/// is proven to be the clean revision the release lock picks for the web player.
fn selected_emulator(root: &Path, emulator: &Path, tools: &Tools) -> Result<(String, String)> {
    // The browser player is the emulator itself, so it may run ahead of the
    // component tuple the disc's launcher is built and checked with.
    let lock = read_json(&root.join("release-components.json"))?;
    let default = lock
        .get("components")
        .and_then(|c| c.get("emulator"))
        .ok_or_else(|| Error("release-components.json has no components.emulator".into()))?;
    let selected = lock.get("web_player").unwrap_or(default);
    let wanted = selected
        .get("revision")
        .and_then(Value::as_str)
        .ok_or_else(|| Error("the selected emulator entry has no revision".into()))?;
    let revision = git(emulator, &["rev-parse", "HEAD"])?;
    let dirty = git(
        emulator,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?;
    ensure!(
        revision == wanted && dirty.is_empty(),
        "web emulator must be the clean revision selected by release-components.json"
    );
    crate::components::bootstrap_tree(&tools.python, emulator, &["--check".to_string()])?;
    let lock_sha256 = sha256_bytes(&fs::read(emulator.join("components.lock.json"))?);
    Ok((revision, lock_sha256))
}

/// `shutil.copy2`: the bytes, the permissions and the modification time.
fn copy2(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination).map_err(|e| Error(format!("{}: {e}", source.display())))?;
    let modified = fs::metadata(source)?.modified()?;
    fs::File::options()
        .write(true)
        .open(destination)?
        .set_modified(modified)?;
    Ok(())
}

/// `shutil.copytree`: symlinks are followed and their targets copied.
fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination).map_err(|e| Error(format!("{}: {e}", destination.display())))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if fs::metadata(entry.path())?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            copy2(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Names directly inside `directory` for which `matches` holds (`Path.glob`
/// with a single-segment pattern).
fn directory_matches(directory: &Path, matches: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if matches(&entry.file_name().to_string_lossy()) {
            found.push(entry.path());
        }
    }
    Ok(found)
}

/// A reused build may carry an older disc, so only the frontend's own assets
/// stay: `demo-disc.*`, `demo-data.*`, `track-*.flac` and `web-manifest.*` go.
fn is_stale_disc_file(name: &str) -> bool {
    name.starts_with("demo-disc.")
        || name.starts_with("demo-data.")
        || name.starts_with("web-manifest.")
        || (name.starts_with("track-")
            && name.ends_with(".flac")
            && name.len() >= "track-.flac".len())
}

/// Python's `repr` of a list of names, for the oversize message.
fn python_list(names: &[String]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("'{n}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// Names of the files under `web` that are `limit` bytes or more.
fn oversized_files(web: &Path, limit: u64) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for (_, path) in files_under(web)? {
        if fs::metadata(&path)?.len() >= limit {
            names.push(
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
    }
    Ok(names)
}

struct Options {
    cue: PathBuf,
    version: String,
    emulator: PathBuf,
    out: PathBuf,
    web_build: Option<PathBuf>,
    publish: bool,
}

fn write_json_file(path: &Path, document: &Value) -> Result<()> {
    fs::write(path, dumps(document, 2, false) + "\n")?;
    Ok(())
}

/// Stage the two packages, and publish them only when `options.publish` is set.
fn stage(root: &Path, options: &Options, tools: &Tools) -> Result<()> {
    let cue = resolve_lenient(&options.cue);
    let image = public_image(&cue)?;
    ensure!(
        valid_version(&options.version)?,
        "provide a release version such as v0.34"
    );
    let out = resolve_lenient(&options.out);
    if out.exists() && fs::read_dir(&out)?.next().is_some() {
        bail!("output must be empty; retain earlier release packages separately");
    }
    fs::create_dir_all(&out)?;
    let (download, web) = (out.join("download"), out.join("web"));
    fs::create_dir(&download)?;
    // Quake's shareware terms require SLICNSE.TXT to accompany its data in every package.
    let licence = root.join("release/SLICNSE.TXT");
    let readme = root.join("release/README.txt");
    for source in [&cue, &image, &readme, &licence] {
        let name = source
            .file_name()
            .ok_or_else(|| Error(format!("no file name: {}", source.display())))?;
        copy2(source, &download.join(name))?;
    }
    let emulator = resolve_lenient(&options.emulator);
    let (emulator_revision, emulator_lock_sha256) = selected_emulator(root, &emulator, tools)?;
    if let Some(reused) = &options.web_build {
        let reused = resolve_lenient(reused);
        verify_web_build(&reused, &emulator_revision, &emulator_lock_sha256)?;
        copy_tree(&reused, &web)?;
    } else {
        let mut trunk = Command::new(&tools.trunk);
        trunk
            .args(["build", "--release", "--public-url", "./", "--dist"])
            .arg(&web)
            .current_dir(emulator.join("emu/crates/frontend"))
            .env_remove("NO_COLOR")
            .env(
                "RUSTFLAGS",
                "-C target-feature=+simd128 -C link-arg=-zstack-size=16777216",
            );
        run_program(&mut trunk)?;
    }
    let has_wasm = !directory_matches(&web, |name| name.ends_with(".wasm"))?.is_empty();
    ensure!(
        web.join("index.html").is_file() && has_wasm,
        "web build is missing its entry page or emulator"
    );
    // A reused build may contain an older disc. Only retain frontend assets.
    for path in directory_matches(&web, is_stale_disc_file)? {
        fs::remove_file(&path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    }
    copy2(&licence, &web.join("SLICNSE.TXT"))?;
    let web_cue = web.join("demo-disc.cue");
    let cue_text = read_text(&cue)?;
    let file_line = Regex::new(r"(?m)^FILE .*$")?;
    let rewritten =
        file_line.replace_all(&cue_text, regex::NoExpand(r#"FILE "demo-disc.bin" BINARY"#));
    fs::write(&web_cue, rewritten.as_bytes())?;
    let mut delivery: Vec<String> = vec![
        web_cue.display().to_string(),
        image.display().to_string(),
        web.display().to_string(),
    ];
    delivery.extend(TRACK_TITLES.iter().map(|t| t.to_string()));
    crate::web::run(&delivery)?;
    // web-delivery decodes every FLAC and reconstructs the original BIN before
    // writing its manifest. Neither public package can silently use an old disc.
    let oversized = oversized_files(&web, ITCH_FILE_LIMIT)?;
    ensure!(
        oversized.is_empty(),
        "itch.io HTML file limit exceeded: {}",
        python_list(&oversized)
    );
    write_json_file(
        &web.join(WEB_RECEIPT),
        &web_record(&web, &emulator_revision, &emulator_lock_sha256)?,
    )?;
    let mut receipt = json!({
        "emulator_components_lock_sha256": emulator_lock_sha256,
        "version": options.version,
        "disc_sha256": sha256_file(&image)?,
        "disc_source": git(root, &["rev-parse", "HEAD"])?,
        "emulator_source": git(&emulator, &["rev-parse", "HEAD"])?,
        "download": DOWNLOAD_TARGET,
        "playable": PLAYABLE_TARGET,
        "published": [],
    });
    let receipt_path = out.join("deployment.json");
    write_json_file(&receipt_path, &receipt)?;
    if options.publish {
        for (key, directory) in [("download", &download), ("playable", &web)] {
            let target = receipt[key].as_str().unwrap_or_default().to_string();
            run_program(
                Command::new(&tools.butler)
                    .arg("push")
                    .arg(directory)
                    .arg(&target)
                    .arg("--userversion")
                    .arg(&options.version),
            )?;
            if let Some(published) = receipt["published"].as_array_mut() {
                published.push(Value::String(key.to_string()));
            }
            write_json_file(&receipt_path, &receipt)?;
        }
        for key in ["download", "playable"] {
            let target = receipt[key].as_str().unwrap_or_default().to_string();
            run_program(Command::new(&tools.butler).arg("status").arg(target))?;
        }
    }
    println!("Both public packages staged: {}", out.display());
    println!("Verify both live itch.io pages. Google Drive is a separate, request-only upload.");
    Ok(())
}

pub fn run(args: &[String]) -> Result<i32> {
    let args = Args::parse(
        args,
        &["cue", "version", "emulator", "out", "web-build"],
        &["publish"],
    )?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let root = repo_root()?;
    let options = Options {
        cue: args.require_path("cue")?,
        version: args.require("version")?.to_string(),
        emulator: args
            .path("emulator")
            .unwrap_or_else(|| root.join("games/PSoXide-emulator")),
        out: args.require_path("out")?,
        web_build: args.path("web-build"),
        publish: args.has("publish"),
    };
    stage(&root, &options, &Tools::real())?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disc::testing::{blank_image, put_user, toc_bytes};
    use crate::disc::SECTOR_BYTES;
    use std::os::unix::fs::PermissionsExt;

    // ----- fixtures: a synthetic public disc and fake tool programs ---------

    fn write_toc(image: &mut [u8], rows: &[(&str, u32, u32, u32)]) {
        let toc = toc_bytes(rows);
        for sector in 0..TOC_SECTORS as usize {
            put_user(
                image,
                TOC_LBA as usize + sector,
                0,
                &toc[sector * USER_DATA_BYTES..(sector + 1) * USER_DATA_BYTES],
            );
        }
    }

    fn public_rows() -> Vec<(&'static str, u32, u32, u32)> {
        PUBLIC_GAMES[..7]
            .iter()
            .map(|name| (*name, 30, 8, 0))
            .collect()
    }

    fn msf(sector: usize) -> String {
        format!(
            "{:02}:{:02}:{:02}",
            sector / 75 / 60,
            sector / 75 % 60,
            sector % 75
        )
    }

    /// 40 data sectors then seven 3-sector audio tracks, as bytes.
    fn public_disc(rows: &[(&str, u32, u32, u32)]) -> Vec<u8> {
        let mut image = blank_image(40 + 7 * 3);
        write_toc(&mut image, rows);
        for (i, byte) in image
            .iter_mut()
            .enumerate()
            .skip(40 * SECTOR_BYTES as usize)
        {
            *byte = (i * 31 % 251) as u8;
        }
        image
    }

    fn public_cue(tracks: usize) -> String {
        let mut cue =
            "FILE \"disc.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n".to_string();
        for n in 2..tracks + 2 {
            cue += &format!(
                "  TRACK {n:02} AUDIO\n    INDEX 01 {}\n",
                msf(40 + (n - 2) * 3)
            );
        }
        cue
    }

    fn script(path: &Path, body: &str) -> OsString {
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        path.as_os_str().to_os_string()
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@example.com",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit_all(dir: &Path) -> String {
        git_in(dir, &["add", "-A"]);
        git_in(dir, &["commit", "-q", "-m", "fixture"]);
        git_in(dir, &["rev-parse", "HEAD"])
    }

    fn have(program: &str) -> bool {
        Command::new(program)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Everything one staging run needs, under one temporary directory.
    struct Fixture {
        dir: tempfile::TempDir,
        revision: String,
        tools: Tools,
    }

    impl Fixture {
        /// An emulator pin that still ships the Python bootstrap.
        fn new() -> Fixture {
            Fixture::with_pin(true)
        }

        /// `old_pin` keeps `tools/bootstrap-components.py`; a new pin has only
        /// the SDK's `psoxide-components` under the ignored `.tools/`.
        fn with_pin(old_pin: bool) -> Fixture {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path();
            // The emulator checkout: a clean git repository with a component lock.
            let emulator = base.join("emulator");
            fs::create_dir_all(emulator.join("emu/crates/frontend")).unwrap();
            let sdk = "c".repeat(40);
            let lock = if old_pin {
                "{\"lock\": 1}\n".to_string()
            } else {
                format!("{{\"components\": {{\"sdk\": {{\"revision\": \"{sdk}\"}}}}}}\n")
            };
            fs::write(emulator.join("components.lock.json"), lock).unwrap();
            fs::write(emulator.join("emu/crates/frontend/.keep"), "").unwrap();
            fs::write(emulator.join(".gitignore"), ".tools/\n").unwrap();
            if old_pin {
                fs::create_dir_all(emulator.join("tools")).unwrap();
                fs::write(emulator.join("tools/bootstrap-components.py"), "# stub\n").unwrap();
            }
            git_in(base, &["init", "-q", "emulator"]);
            let revision = commit_all(&emulator);
            // The disc repository root: release lock, README and the Quake licence.
            let root = base.join("root");
            fs::create_dir_all(root.join("release")).unwrap();
            fs::write(root.join("release/README.txt"), "readme\n").unwrap();
            fs::write(root.join("release/SLICNSE.TXT"), "quake licence\n").unwrap();
            fs::write(
                root.join("release-components.json"),
                format!("{{\"components\": {{\"emulator\": {{\"revision\": \"{revision}\"}}}}}}"),
            )
            .unwrap();
            git_in(base, &["init", "-q", "root"]);
            commit_all(&root);
            // The public disc.
            fs::create_dir_all(base.join("disc")).unwrap();
            fs::write(base.join("disc/disc.bin"), public_disc(&public_rows())).unwrap();
            fs::write(base.join("disc/disc.cue"), public_cue(7)).unwrap();
            // Fake programs: each logs what it was called with.
            let bin = base.join("bin");
            fs::create_dir_all(&bin).unwrap();
            let log = |name: &str| base.join(format!("{name}.log"));
            let python = script(
                &bin.join("python3"),
                &format!("echo \"$@\" >> '{}'", log("python").display()),
            );
            if !old_pin {
                fs::create_dir_all(emulator.join(format!(".tools/sdk-{sdk}/bin"))).unwrap();
                script(
                    &emulator.join(format!(".tools/sdk-{sdk}/bin/psoxide-components")),
                    &format!("echo \"$@\" >> '{}'", log("components").display()),
                );
            }
            let trunk = script(
                &bin.join("trunk"),
                &format!(
                    "echo \"$PWD|$RUSTFLAGS|${{NO_COLOR-unset}}|$*\" >> '{log}'\n\
                     while [ $# -gt 0 ]; do [ \"$1\" = --dist ] && dist=\"$2\"; shift; done\n\
                     mkdir -p \"$dist\"\n\
                     echo '<html></html>' > \"$dist/index.html\"\n\
                     printf wasm > \"$dist/app.wasm\"\n\
                     printf old > \"$dist/demo-disc.bin\"\n\
                     printf old > \"$dist/track-09.flac\"\n\
                     printf old > \"$dist/web-manifest.json\"\n\
                     printf css > \"$dist/style.css\"",
                    log = log("trunk").display()
                ),
            );
            let butler = script(
                &bin.join("butler"),
                &format!("echo \"$@\" >> '{}'", log("butler").display()),
            );
            Fixture {
                dir,
                revision,
                tools: Tools {
                    python,
                    trunk,
                    butler,
                },
            }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.dir.path().join(relative)
        }

        fn options(&self, out: &str) -> Options {
            Options {
                cue: self.path("disc/disc.cue"),
                version: "v1.2".to_string(),
                emulator: self.path("emulator"),
                out: self.path(out),
                web_build: None,
                publish: false,
            }
        }

        fn stage(&self, options: &Options) -> Result<()> {
            stage(&self.path("root"), options, &self.tools)
        }

        fn log(&self, name: &str) -> Option<String> {
            fs::read_to_string(self.path(&format!("{name}.log"))).ok()
        }
    }

    // ----- version, layout and receipts --------------------------------------

    #[test]
    fn version_accepts_release_names_only() {
        for good in [
            "v0.34",
            "v1.2.3",
            "v0.34-rc.1",
            "v10.20.30",
            "v0.34.beta2",
            "v1.0-x",
            "v2.5.0.1",
        ] {
            assert!(valid_version(good).unwrap(), "{good} should be accepted");
        }
        for bad in [
            "",
            "0.34",
            "v0",
            "v1",
            "v.5",
            "v1.",
            "V1.2",
            "v1.2.3.4-",
            "v1.2 ",
            " v1.2",
            "v1.2\n",
            "v1.2-",
            "v1.2/x",
            "v1.2;rm",
            "latest",
            "v1.2-é",
            "v1.2.x y",
            "v1,2",
        ] {
            assert!(!valid_version(bad).unwrap(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn public_toc_menu_hides_hidden_entries_and_appends_credits() {
        let toc = parse_public_toc(&toc_bytes(&[
            ("A", 1, 1, 0),
            ("B", 2, 2, 1),
            ("C", 3, 3, 0),
        ]))
        .unwrap();
        assert_eq!((toc.claimed, toc.visible), (3, 2));
        assert_eq!(toc.menu, ["A", "C", "CREDITS"]);
        // Every entry hidden: no CREDITS card either.
        let none = parse_public_toc(&toc_bytes(&[("A", 1, 1, 1)])).unwrap();
        assert!(none.menu.is_empty());
        assert!(parse_public_toc(&vec![0u8; TOC_SECTORS as usize * USER_DATA_BYTES]).is_err());
    }

    fn adjacent_disc(dir: &Path, cue: &str, rows: &[(&str, u32, u32, u32)]) -> PathBuf {
        fs::write(dir.join("disc.bin"), public_disc(rows)).unwrap();
        fs::write(dir.join("disc.cue"), cue).unwrap();
        dir.join("disc.cue")
    }

    #[test]
    fn public_image_accepts_exactly_the_public_collection() {
        let dir = tempfile::tempdir().unwrap();
        let cue = adjacent_disc(dir.path(), &public_cue(7), &public_rows());
        assert_eq!(public_image(&cue).unwrap(), dir.path().join("disc.bin"));
        // A CRLF cue reads the same (Python's read_text translates newlines).
        let crlf = adjacent_disc(
            dir.path(),
            &public_cue(7).replace('\n', "\r\n"),
            &public_rows(),
        );
        assert!(public_image(&crlf).is_ok());
    }

    #[test]
    fn public_image_refuses_anything_else() {
        let dir = tempfile::tempdir().unwrap();
        let message = |cue: &str, rows: &[(&str, u32, u32, u32)]| {
            public_image(&adjacent_disc(dir.path(), cue, rows))
                .unwrap_err()
                .0
        };
        // Wrong menu: a card swapped, one missing, one extra, one hidden.
        let mut swapped = public_rows();
        swapped[2].0 = "SOMETHING ELSE";
        assert!(message(&public_cue(7), &swapped).contains("does not match the public collection"));
        assert!(message(&public_cue(7), &public_rows()[..6])
            .contains("does not match the public collection"));
        let mut extra = public_rows();
        extra.push(("HALF-LIFE", 1, 1, 0));
        assert!(message(&public_cue(7), &extra).contains("does not match the public collection"));
        let mut hidden = public_rows();
        hidden[3].3 = FLAG_HIDDEN;
        assert!(message(&public_cue(7), &hidden).contains("does not match the public collection"));
        // A reordered menu is not the public menu either.
        let mut reordered = public_rows();
        reordered.swap(0, 1);
        assert!(
            message(&public_cue(7), &reordered).contains("does not match the public collection")
        );
        // The entry count the table claims must be right even when the menu is.
        let mut padded = public_rows();
        padded.push(("EXTRA HIDDEN", 1, 1, FLAG_HIDDEN));
        assert!(message(&public_cue(7), &padded).contains("does not match the public collection"));
        // Wrong track layout.
        assert!(message(&public_cue(6), &public_rows()).contains("8-track layout"));
        assert!(message(&public_cue(8), &public_rows()).contains("8-track layout"));
        let data_audio = public_cue(7).replace("TRACK 03 AUDIO", "TRACK 03 MODE2/2352");
        assert!(message(&data_audio, &public_rows()).contains("8-track layout"));
        // BIN naming.
        let two = format!("{}FILE \"other.bin\" BINARY\n", public_cue(7));
        assert!(message(&two, &public_rows()).contains("one adjacent BIN"));
        let nested = public_cue(7).replace("\"disc.bin\"", "\"sub/disc.bin\"");
        assert!(message(&nested, &public_rows()).contains("one adjacent BIN"));
        let none = public_cue(7).replace("FILE \"disc.bin\" BINARY", "");
        assert!(message(&none, &public_rows()).contains("one adjacent BIN"));
    }

    #[test]
    fn web_build_receipt_requires_matching_source_and_files() {
        // The port of test_deploy_public.py.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("frontend.wasm"), b"wasm fixture").unwrap();
        let (a, b, c) = ("a".repeat(40), "b".repeat(64), "c".repeat(40));
        assert!(verify_web_build(root, &a, &b)
            .unwrap_err()
            .0
            .contains("no emulator source"));
        write_json_file(&root.join(WEB_RECEIPT), &web_record(root, &a, &b).unwrap()).unwrap();
        verify_web_build(root, &a, &b).unwrap();
        assert!(verify_web_build(root, &c, &b)
            .unwrap_err()
            .0
            .contains("differs"));
        assert!(verify_web_build(root, &a, &"d".repeat(64))
            .unwrap_err()
            .0
            .contains("differs"));
        fs::write(root.join("frontend.wasm"), b"changed").unwrap();
        assert!(verify_web_build(root, &a, &b)
            .unwrap_err()
            .0
            .contains("differs"));
    }

    #[test]
    fn web_record_lists_every_file_in_path_order_without_the_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("deep/er")).unwrap();
        fs::write(root.join("a/b"), "1").unwrap();
        fs::write(root.join("a-c"), "2").unwrap();
        fs::write(root.join("deep/er/x.js"), "3").unwrap();
        fs::write(root.join(WEB_RECEIPT), "ignored").unwrap();
        fs::write(root.join("deep").join(WEB_RECEIPT), "ignored too").unwrap();
        let record = web_record(root, &"a".repeat(40), &"b".repeat(64)).unwrap();
        let keys: Vec<&String> = record["files"].as_object().unwrap().keys().collect();
        // Components compare one by one, so a/b sorts before a-c.
        assert_eq!(keys, ["a/b", "a-c", "deep/er/x.js"]);
        assert_eq!(record["schema"], 1);
        assert_eq!(record["files"]["a-c"], sha256_bytes(b"2"));
        let text = dumps(&record, 2, false);
        assert!(text.starts_with("{\n  \"schema\": 1,\n  \"emulator_revision\": "));
        // A receipt with its keys in another order still verifies, like dict equality.
        let reordered = json!({
            "files": record["files"].clone(),
            "components_lock_sha256": record["components_lock_sha256"].clone(),
            "emulator_revision": record["emulator_revision"].clone(),
            "schema": 1,
        });
        write_json_file(&root.join(WEB_RECEIPT), &reordered).unwrap();
        verify_web_build(root, &"a".repeat(40), &"b".repeat(64)).unwrap();
    }

    #[test]
    fn stale_disc_files_are_the_ones_the_original_globbed() {
        for stale in [
            "demo-disc.bin",
            "demo-disc.cue",
            "demo-data.bin.gz",
            "track-02.flac",
            "track-.flac",
            "web-manifest.txt",
            "web-manifest.json",
        ] {
            assert!(is_stale_disc_file(stale), "{stale}");
        }
        for kept in [
            "index.html",
            "app.wasm",
            "track-02.mp3",
            "demo-disc",
            "track.flac",
            "SLICNSE.TXT",
            "web-manifest",
        ] {
            assert!(!is_stale_disc_file(kept), "{kept}");
        }
    }

    #[test]
    fn oversized_files_are_found_by_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("small.bin"), vec![0u8; 99]).unwrap();
        fs::write(dir.path().join("sub/big.bin"), vec![0u8; 100]).unwrap();
        assert_eq!(oversized_files(dir.path(), 100).unwrap(), ["big.bin"]);
        assert!(oversized_files(dir.path(), 101).unwrap().is_empty());
        assert_eq!(python_list(&["a".into(), "b".into()]), "['a', 'b']");
        assert_eq!(ITCH_FILE_LIMIT, 200_000_000);
    }

    // ----- the staging run -----------------------------------------------------

    #[test]
    fn refusals_happen_in_order_and_before_anything_is_written() {
        let f = Fixture::new();
        // A bad version stops the run before the output directory exists.
        let mut options = f.options("out");
        options.version = "0.34".to_string();
        assert!(f.stage(&options).unwrap_err().0.contains("release version"));
        assert!(!f.path("out").exists());
        // A missing cue, and a non-public disc, are refused before the version
        // is even looked at.
        let mut options = f.options("out");
        options.version = "nonsense".to_string();
        options.cue = f.path("disc/missing.cue");
        assert!(f.stage(&options).is_err());
        fs::write(f.path("disc/disc.cue"), public_cue(3)).unwrap();
        options.cue = f.path("disc/disc.cue");
        assert!(f.stage(&options).unwrap_err().0.contains("track layout"));
        assert!(!f.path("out").exists());
        // A non-empty output directory is refused, and left as it was.
        fs::write(f.path("disc/disc.cue"), public_cue(7)).unwrap();
        fs::create_dir_all(f.path("busy")).unwrap();
        fs::write(f.path("busy/earlier-release.txt"), "keep me").unwrap();
        let error = f.stage(&f.options("busy")).unwrap_err();
        assert!(error.0.contains("output must be empty"), "{error}");
        assert_eq!(fs::read_dir(f.path("busy")).unwrap().count(), 1);
        assert!(f.log("trunk").is_none() && f.log("butler").is_none());
        // An existing but empty directory is fine to stage into.
        fs::create_dir_all(f.path("empty")).unwrap();
        assert!(f.stage(&f.options("empty")).is_ok() || !have("flac"));
    }

    #[test]
    fn staging_builds_both_packages_and_publishes_nothing() {
        if !have("flac") || !have("git") {
            eprintln!("skipping: flac or git is not installed");
            return;
        }
        std::env::set_var("NO_COLOR", "1");
        let f = Fixture::new();
        f.stage(&f.options("out")).unwrap();
        let out = resolve(&f.path("out")).unwrap();
        // The download package: cue, BIN, README and the Quake licence.
        let mut names: Vec<String> = fs::read_dir(out.join("download"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["README.txt", "SLICNSE.TXT", "disc.bin", "disc.cue"]);
        assert_eq!(
            fs::read_to_string(out.join("download/SLICNSE.TXT")).unwrap(),
            "quake licence\n"
        );
        // The web package: the licence too, the fresh delivery, no stale disc files.
        assert_eq!(
            fs::read_to_string(out.join("web/SLICNSE.TXT")).unwrap(),
            "quake licence\n"
        );
        for kept in [
            "index.html",
            "app.wasm",
            "style.css",
            "demo-data.bin.gz",
            "web-manifest.txt",
            "web-manifest.json",
            "demo-disc.cue",
            "emulator-build.json",
        ] {
            assert!(
                out.join("web").join(kept).is_file(),
                "{kept} should be in the web package"
            );
        }
        assert!(!out.join("web/demo-disc.bin").exists());
        assert!(!out.join("web/track-09.flac").exists());
        assert!(out.join("web/track-02.flac").is_file());
        let web_cue = fs::read_to_string(out.join("web/demo-disc.cue")).unwrap();
        assert!(web_cue.starts_with("FILE \"demo-disc.bin\" BINARY\n  TRACK 01 MODE2/2352"));
        let manifest = fs::read_to_string(out.join("web/web-manifest.txt")).unwrap();
        assert!(
            manifest.contains(" KNUCKLE DUST\n")
                && manifest.trim_end().ends_with("CORTEX IGNITION MENU")
        );
        // The receipt matches the locked emulator and the files that are there.
        let lock_sha = sha256_bytes(b"{\"lock\": 1}\n");
        verify_web_build(&out.join("web"), &f.revision, &lock_sha).unwrap();
        let deployment = read_json(&out.join("deployment.json")).unwrap();
        assert_eq!(deployment["version"], "v1.2");
        assert_eq!(deployment["emulator_source"], f.revision.as_str());
        assert_eq!(
            deployment["emulator_components_lock_sha256"],
            lock_sha.as_str()
        );
        assert_eq!(
            deployment["disc_sha256"],
            sha256_file(&f.path("disc/disc.bin")).unwrap().as_str()
        );
        assert_eq!(
            deployment["disc_source"],
            git_in(&f.path("root"), &["rev-parse", "HEAD"]).as_str()
        );
        assert_eq!(
            deployment["download"],
            "bonnie-studios/psoxide-demo-disc:psx"
        );
        assert_eq!(deployment["playable"], "bonnie-studios/psoxide:html5");
        assert_eq!(deployment["published"], json!([]));
        let text = fs::read_to_string(out.join("deployment.json")).unwrap();
        assert!(
            text.starts_with("{\n  \"emulator_components_lock_sha256\": "),
            "keys keep insertion order"
        );
        assert!(text.ends_with("}\n"));
        // The emulator's own check ran, and trunk ran as before; butler never did.
        let emulator = resolve(&f.path("emulator")).unwrap();
        assert_eq!(
            f.log("python").unwrap().trim(),
            format!(
                "{} --check",
                emulator.join("tools/bootstrap-components.py").display()
            )
        );
        let trunk = f.log("trunk").unwrap();
        let (cwd, rest) = trunk.trim().split_once('|').unwrap();
        assert_eq!(
            cwd,
            resolve(&emulator.join("emu/crates/frontend"))
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(
            rest,
            format!(
                "-C target-feature=+simd128 -C link-arg=-zstack-size=16777216|unset|build --release --public-url ./ --dist {}",
                out.join("web").display()
            )
        );
        assert!(
            f.log("butler").is_none(),
            "nothing may be uploaded without --publish"
        );
    }

    #[test]
    fn publish_pushes_download_then_web_and_checks_status() {
        if !have("flac") || !have("git") {
            eprintln!("skipping: flac or git is not installed");
            return;
        }
        let f = Fixture::new();
        let mut options = f.options("out");
        options.publish = true;
        f.stage(&options).unwrap();
        let out = resolve(&f.path("out")).unwrap();
        let calls: Vec<String> = f
            .log("butler")
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(
            calls,
            [
                format!(
                    "push {} bonnie-studios/psoxide-demo-disc:psx --userversion v1.2",
                    out.join("download").display()
                ),
                format!(
                    "push {} bonnie-studios/psoxide:html5 --userversion v1.2",
                    out.join("web").display()
                ),
                "status bonnie-studios/psoxide-demo-disc:psx".to_string(),
                "status bonnie-studios/psoxide:html5".to_string(),
            ]
        );
        let deployment = read_json(&out.join("deployment.json")).unwrap();
        assert_eq!(deployment["published"], json!(["download", "playable"]));
    }

    #[test]
    fn a_failed_push_stops_before_the_second_upload() {
        if !have("flac") || !have("git") {
            return;
        }
        let mut f = Fixture::new();
        let log = f.path("butler.log");
        f.tools.butler = script(
            &f.path("bin/butler"),
            &format!(
                "echo \"$@\" >> '{}'\n[ \"$1\" = push ] && exit 1\nexit 0",
                log.display()
            ),
        );
        let mut options = f.options("out");
        options.publish = true;
        assert!(f.stage(&options).is_err());
        assert_eq!(
            f.log("butler").unwrap().lines().count(),
            1,
            "the web package must not be pushed after a failure"
        );
        let deployment = read_json(&f.path("out/deployment.json")).unwrap();
        assert_eq!(deployment["published"], json!([]));
    }

    #[test]
    fn the_emulator_must_be_the_clean_selected_revision() {
        if !have("flac") || !have("git") {
            return;
        }
        // A dirty checkout (even an untracked file) is refused.
        let f = Fixture::new();
        fs::write(f.path("emulator/scratch.txt"), "untracked").unwrap();
        let error = f.stage(&f.options("out")).unwrap_err();
        assert!(
            error
                .0
                .contains("clean revision selected by release-components.json"),
            "{error}"
        );
        assert!(f.log("trunk").is_none() && f.log("python").is_none());
        // A checkout at another revision is refused.
        let f = Fixture::new();
        fs::write(f.path("emulator/more.txt"), "x").unwrap();
        commit_all(&f.path("emulator"));
        let error = f.stage(&f.options("out")).unwrap_err();
        assert!(error.0.contains("clean revision"), "{error}");
        // web_player, when present, wins over components.emulator.
        let moved = git_in(&f.path("emulator"), &["rev-parse", "HEAD"]);
        fs::write(
            f.path("root/release-components.json"),
            format!(
                "{{\"components\": {{\"emulator\": {{\"revision\": \"{}\"}}}}, \"web_player\": {{\"revision\": \"{moved}\"}}}}",
                f.revision
            ),
        )
        .unwrap();
        f.stage(&f.options("out2")).unwrap();
        // The emulator's own component check failing stops the run before trunk.
        let mut f = Fixture::new();
        f.tools.python = script(&f.path("bin/python3"), "exit 3");
        let error = f.stage(&f.options("out")).unwrap_err();
        assert!(error.0.contains("failed"), "{error}");
        assert!(f.log("trunk").is_none() && f.log("butler").is_none());
        let mut options = f.options("out3");
        options.publish = true;
        assert!(f.stage(&options).is_err());
        assert!(f.log("butler").is_none());
    }

    #[test]
    fn a_new_emulator_pin_is_checked_with_psoxide_components() {
        if !have("flac") || !have("git") {
            return;
        }
        let f = Fixture::with_pin(false);
        f.stage(&f.options("out")).unwrap();
        let emulator = resolve(&f.path("emulator")).unwrap();
        assert_eq!(
            f.log("components").unwrap().trim(),
            format!("--root {} --check", emulator.display())
        );
        // No Python was involved, and trunk still ran after the check.
        assert!(f.log("python").is_none());
        assert!(f.log("trunk").is_some());
    }

    #[test]
    fn a_reused_web_build_needs_a_matching_receipt() {
        if !have("flac") || !have("git") {
            return;
        }
        let f = Fixture::new();
        let reused = f.path("reused");
        fs::create_dir_all(reused.join("assets")).unwrap();
        fs::write(reused.join("index.html"), "<html></html>").unwrap();
        fs::write(reused.join("app.wasm"), "wasm").unwrap();
        fs::write(reused.join("assets/frontend.js"), "js").unwrap();
        fs::write(reused.join("demo-disc.bin"), "an older disc").unwrap();
        let mut options = f.options("out");
        options.web_build = Some(reused.clone());
        options.publish = true;
        // No receipt: refused, nothing uploaded, and trunk is not used instead.
        let error = f.stage(&options).unwrap_err();
        assert!(
            error.0.contains("no emulator source and content receipt"),
            "{error}"
        );
        // A receipt for another revision is refused too.
        let lock_sha = sha256_bytes(b"{\"lock\": 1}\n");
        write_json_file(
            &reused.join(WEB_RECEIPT),
            &web_record(&reused, &"e".repeat(40), &lock_sha).unwrap(),
        )
        .unwrap();
        options.out = f.path("out2");
        assert!(f.stage(&options).unwrap_err().0.contains("differs"));
        assert!(f.log("trunk").is_none() && f.log("butler").is_none());
        // A matching receipt is accepted; the older disc is dropped from the copy only.
        write_json_file(
            &reused.join(WEB_RECEIPT),
            &web_record(&reused, &f.revision, &lock_sha).unwrap(),
        )
        .unwrap();
        options.out = f.path("out3");
        f.stage(&options).unwrap();
        assert!(
            f.log("trunk").is_none(),
            "a reused build must not be rebuilt"
        );
        let out = resolve(&f.path("out3")).unwrap();
        assert!(out.join("web/assets/frontend.js").is_file());
        assert!(!out.join("web/demo-disc.bin").exists());
        assert!(
            reused.join("demo-disc.bin").exists(),
            "the source build is left untouched"
        );
        assert_eq!(f.log("butler").unwrap().lines().count(), 4);
    }

    #[test]
    fn a_web_build_without_an_entry_page_or_wasm_is_refused() {
        if !have("git") {
            return;
        }
        let f = Fixture::new();
        let reused = f.path("reused");
        fs::create_dir_all(&reused).unwrap();
        fs::write(reused.join("index.html"), "x").unwrap();
        let lock_sha = sha256_bytes(b"{\"lock\": 1}\n");
        write_json_file(
            &reused.join(WEB_RECEIPT),
            &web_record(&reused, &f.revision, &lock_sha).unwrap(),
        )
        .unwrap();
        let mut options = f.options("out");
        options.web_build = Some(reused);
        let error = f.stage(&options).unwrap_err();
        assert!(
            error.0.contains("missing its entry page or emulator"),
            "{error}"
        );
    }

    #[test]
    fn options_are_required() {
        assert!(run(&[]).unwrap_err().0.contains("--cue is required"));
        let args: Vec<String> = ["--cue", "x"].map(String::from).to_vec();
        assert!(run(&args).unwrap_err().0.contains("--version is required"));
        let args: Vec<String> = ["--cue", "x", "--version", "v1.0"]
            .map(String::from)
            .to_vec();
        assert!(run(&args).unwrap_err().0.contains("--out is required"));
        let args: Vec<String> = ["--cue", "x", "--version", "v1.0", "--out", "y", "--upload"]
            .map(String::from)
            .to_vec();
        assert!(run(&args).is_err());
    }
}
