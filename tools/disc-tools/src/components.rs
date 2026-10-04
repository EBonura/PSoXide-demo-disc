//! Verify and bootstrap the exact repositories selected for this demo disc.
//!
//! `release-components.json` pins the SDK, editor and emulator revisions. This
//! checks that the submodule checkouts are those revisions and clean, that the
//! pins nested inside the editor and emulator agree with the disc's, and (when
//! asked) that every standalone game and its hydrated build inputs agree too.
//!
//! The bootstrap itself, materialising the pinned sources where Cargo expects
//! them, is `tools/bootstrap-components.py` inside the editor and emulator
//! repositories. It belongs to those repositories and is run as they ship it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

use crate::args::Args;
use crate::disc::image_for_cue;
use crate::util::{
    command_output, file_record, git, read_json, repo_root, sha256_bytes, write_receipt, Error,
    Result,
};

const GAMES: [&str; 5] = [
    "voxide",
    "nitroxide",
    "psxcel",
    "pico8-psx",
    "psoxide-arcade",
];
/// The optional bring-your-own-assets ports. hl-psx and cs-psx hydrate the
/// shared editor tree like every other game; hk-psx imports only the SDK,
/// through its own sdk.lock.json, so its lock is checked by revision instead.
const OPTIONAL: [(&str, &str); 2] = [("hl", "hl-psx"), ("cs", "cs-psx")];
const HK: &str = "hk-psx";

type GitFn<'a> = &'a dyn Fn(&Path, &[&str]) -> Result<String>;
type BootstrapFn<'a> = &'a dyn Fn(&Path, &[String]) -> Result<()>;

/// What a check reads from the world: the repository root and how to ask git.
pub struct Env<'a> {
    pub root: PathBuf,
    pub git: GitFn<'a>,
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error(format!("'{key}'")))
}

fn components(env: &Env) -> Result<Map<String, Value>> {
    let lock = read_json(&env.root.join("release-components.json"))?;
    lock.get("components")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| Error("release-components.json has no components".into()))
}

fn gh_compare(repository: &str, revision: &str) -> Result<String> {
    command_output(
        "gh",
        &[
            "api",
            &format!("repos/{repository}/compare/{revision}...main"),
            "--jq",
            ".status",
        ],
        None,
    )
}

/// The component tuple: checkouts at their pins and clean, nested pins agreeing,
/// then the sub-repositories' own bootstrap run (or, with `check`, verified).
pub fn run_checks(env: &Env, check: bool, check_main: bool, bootstrap: BootstrapFn) -> Result<()> {
    let lock = read_json(&env.root.join("release-components.json"))?;
    ensure!(
        lock.get("schema") == Some(&json!(1)),
        "Unsupported component lock"
    );
    let specs = components(env)?;
    for (name, spec) in &specs {
        let source = env.root.join(field(spec, "path")?);
        ensure!(
            (env.git)(&source, &["rev-parse", "HEAD"])? == field(spec, "revision")?,
            "{name}: checkout does not match release-components.json"
        );
        ensure!(
            (env.git)(
                &source,
                &["status", "--porcelain", "--untracked-files=normal"]
            )?
            .is_empty(),
            "{name}: source checkout is dirty"
        );
        if check_main {
            let status = gh_compare(field(spec, "repository")?, field(spec, "revision")?)?;
            ensure!(
                status == "identical" || status == "ahead",
                "{name}: selected revision is not on its repository main"
            );
        }
    }
    for (owner, dependencies) in [
        ("editor", &["sdk", "emulator"][..]),
        ("emulator", &["sdk"][..]),
    ] {
        let source = env.root.join(field(&specs[owner], "path")?);
        let nested = read_json(&source.join("components.lock.json"))?;
        for name in dependencies {
            let pinned = nested["components"][name]["revision"].as_str();
            ensure!(
                pinned == Some(field(&specs[*name], "revision")?),
                "{owner} {name} pin differs from the disc component lock"
            );
        }
    }
    for name in ["emulator", "editor"] {
        let source = env.root.join(field(&specs[name], "path")?);
        let mut command: Vec<String> = vec![source
            .join("tools/bootstrap-components.py")
            .display()
            .to_string()];
        if check || check_main {
            command.push("--check".into());
        } else {
            command.push("--source".into());
            command.push(format!(
                "sdk={}",
                env.root.join(field(&specs["sdk"], "path")?).display()
            ));
            command.push("--source".into());
            command.push(format!(
                "emulator={}",
                env.root.join(field(&specs["emulator"], "path")?).display()
            ));
        }
        bootstrap(&env.root, &command)?;
    }
    println!("Release component revisions and imported source receipts verified");
    Ok(())
}

/// Run a sub-repository's bootstrap script with the interpreter on PATH
/// (`PYTHON` overrides it).
fn run_bootstrap(_root: &Path, command: &[String]) -> Result<()> {
    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
    let status = Command::new(&python)
        .args(command)
        .status()
        .map_err(|e| Error(format!("cannot run {python}: {e}")))?;
    ensure!(
        status.success(),
        "{} {} exited with {status}",
        python,
        command.join(" ")
    );
    Ok(())
}

/// hk-psx imports only the SDK (and records the emulator), each through its own
/// lock file, which has to name the disc's revision of that repository.
fn verify_hk_locks(env: &Env, specs: &Map<String, Value>) -> Result<()> {
    let source = env.root.join("games").join(HK);
    for (lock, name) in [("sdk.lock.json", "sdk"), ("emulator.lock.json", "emulator")] {
        let path = source.join(lock);
        ensure!(path.is_file(), "{HK}: no {lock}");
        let pinned = read_json(&path)?;
        ensure!(
            pinned["revision"] == specs[name]["revision"],
            "{HK}: {lock} revision differs from the disc {name} lock"
        );
        let repository = field(&pinned, "repository")?;
        let wanted = field(&specs[name], "repository")?;
        ensure!(
            repository
                .strip_suffix(".git")
                .unwrap_or(repository)
                .ends_with(wanted),
            "{HK}: {lock} repository differs from the disc {name} lock"
        );
    }
    Ok(())
}

/// hk-psx hydrates .psoxide from `git archive` of its SDK pin and records it.
fn verify_hk_hydration(env: &Env, specs: &Map<String, Value>) -> Result<()> {
    let state = env.root.join("games").join(HK).join(".hkpsx/sdk.json");
    ensure!(
        state.is_file(),
        "{HK}: missing SDK hydration record; run make programs HK=1"
    );
    ensure!(
        read_json(&state)?["revision"] == specs["sdk"]["revision"],
        "{HK}: hydrated SDK differs from the disc sdk lock"
    );
    Ok(())
}

pub fn verify_game_locks(env: &Env) -> Result<()> {
    let specs = components(env)?;
    verify_hk_locks(env, &specs)?;
    let optional = OPTIONAL.iter().map(|(_, game)| *game);
    for game in GAMES.iter().copied().chain(optional) {
        let source = env.root.join("games").join(game);
        let lock = source.join("components.lock.json");
        ensure!(
            lock.is_file(),
            "{game}: no components.lock.json; every game must import PSoXide from the lock"
        );
        let document = read_json(&lock)?;
        for name in ["sdk", "emulator", "editor"] {
            for key in ["repository", "revision"] {
                ensure!(
                    document["components"][name][key] == specs[name][key],
                    "{game}: standalone {name} {key} differs from the disc lock"
                );
            }
        }
    }
    println!("Every standalone game lock agrees with the release component tuple");
    Ok(())
}

/// Every hydrated game tree carries the editor's imported and owned build
/// inputs unchanged, from the shared source named in its marker.
pub fn verify_games(env: &Env, hl: bool, cs: bool, hk: bool) -> Result<()> {
    verify_game_locks(env)?;
    let specs = components(env)?;
    let editor = env.root.join(field(&specs["editor"], "path")?);
    let receipt = read_json(&editor.join(".components-receipt.json"))?;
    // Compare both imported SDK/emulator files and editor-owned build inputs.
    let mut inputs: Vec<(String, String)> = receipt["files"]
        .as_object()
        .map(|files| {
            files
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    for name in (env.git)(&editor, &["ls-files"])?.lines() {
        let path = editor.join(name);
        let owned = ["engine/", "editor/crates/", "sdk/", "crates/"]
            .iter()
            .any(|p| name.starts_with(p));
        if owned && path.is_file() {
            let digest = sha256_bytes(&fs::read(&path)?);
            match inputs.iter_mut().find(|(k, _)| k == name) {
                Some(slot) => slot.1 = digest,
                None => inputs.push((name.to_string(), digest)),
            }
        }
    }
    let mut selected: Vec<&str> = GAMES.to_vec();
    for (flag, game) in OPTIONAL {
        if (flag == "hl" && hl) || (flag == "cs" && cs) {
            selected.push(game);
        }
    }
    if hk {
        verify_hk_hydration(env, &specs)?;
    }
    for game in &selected {
        let hydrated = env.root.join("games").join(game).join(".psoxide");
        let marker = hydrated.join(".psoxide-source");
        ensure!(
            marker.is_file()
                && fs::read_to_string(&marker)? == format!("local:{}", editor.display()),
            "{game}: missing current shared-source hydration; run make programs"
        );
        for (name, expected) in &inputs {
            let path = hydrated.join(name);
            let intact = path.is_file()
                && !fs::symlink_metadata(&path)?.file_type().is_symlink()
                && sha256_bytes(&fs::read(&path)?) == *expected;
            ensure!(
                intact,
                "{game}: hydrated build input changed or missing: {name}"
            );
        }
    }
    println!(
        "Verified all {} required game hydrations against the selected components",
        selected.len() + usize::from(hk)
    );
    Ok(())
}

/// The component receipt for a pressed `make disc`: every submodule at its
/// gitlink and clean, with the lock and receipt files that identify it.
pub fn write_component_receipt(
    env: &Env,
    output: &Path,
    cue: &Path,
    frontend: &Path,
) -> Result<()> {
    let root = &env.root;
    let mut repositories = Map::new();
    for line in (env.git)(root, &["ls-files", "--stage"])?.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(mode), Some(revision), Some(rest)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if mode != "160000" {
            continue;
        }
        let path = rest.split_once('\t').map(|(_, p)| p).unwrap_or(rest);
        let source = root.join(path);
        ensure!(
            (env.git)(&source, &["rev-parse", "HEAD"])? == revision,
            "{path}: working revision differs from the selected gitlink"
        );
        ensure!(
            (env.git)(
                &source,
                &["status", "--porcelain", "--untracked-files=normal"]
            )?
            .is_empty(),
            "{path}: source is dirty"
        );
        let mut row = Map::new();
        row.insert("revision".into(), json!(revision));
        for name in [
            "components.lock.json",
            ".components-receipt.json",
            ".psoxide/.components-receipt.json",
            ".psoxide/.psoxide-source",
        ] {
            if source.join(name).is_file() {
                row.insert(name.into(), file_record(&source.join(name))?);
            }
        }
        repositories.insert(path.to_string(), Value::Object(row));
    }
    let lock = read_json(&root.join("release-components.json"))?;
    let document = json!({
        "schema": 1,
        "components": lock["components"],
        "repositories": repositories,
        "demo_revision": (env.git)(root, &["rev-parse", "HEAD"])?,
        "build_recipe": file_record(&root.join("Makefile"))?,
        "rustc": command_output("rustc", &["--version"], Some(root))?,
        "cargo": command_output("cargo", &["--version"], Some(root))?,
        "frontend": file_record(frontend)?,
        "cue": file_record(cue)?,
        "bin": file_record(&image_for_cue(cue)?)?,
    });
    write_receipt(output, &document)?;
    println!("Complete component/game provenance: {}", output.display());
    Ok(())
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(
        raw,
        &["receipt", "cue", "frontend"],
        &[
            "check",
            "check-main",
            "games",
            "game-locks",
            "hl",
            "cs",
            "hk",
        ],
    )?;
    let root = repo_root()?.canonicalize()?;
    let real_git = |dir: &Path, a: &[&str]| git(dir, a);
    let env = Env {
        root,
        git: &real_git,
    };
    let outcome = (|| -> Result<()> {
        run_checks(
            &env,
            args.has("check"),
            args.has("check-main"),
            &run_bootstrap,
        )?;
        if args.has("games") {
            verify_games(&env, args.has("hl"), args.has("cs"), args.has("hk"))?;
        } else if args.has("game-locks") {
            verify_game_locks(&env)?;
        }
        if let Some(receipt) = args.path("receipt") {
            let (Some(cue), Some(frontend)) = (args.path("cue"), args.path("frontend")) else {
                bail!("--receipt requires --cue and --frontend");
            };
            write_component_receipt(&env, &receipt, &cue, &frontend)?;
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => Ok(0),
        Err(error) => {
            eprintln!("components: {error}");
            Ok(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn specs() -> Value {
        let mut map = Map::new();
        for (name, letter) in [("sdk", 'a'), ("emulator", 'b'), ("editor", 'c')] {
            map.insert(
                name.into(),
                json!({"revision": letter.to_string().repeat(40), "repository": format!("owner/{name}"), "path": name}),
            );
        }
        Value::Object(map)
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// Every game, hydrated copy and lock the checks look at, all consistent.
    fn fixture(root: &Path) {
        let specs = specs();
        write(
            &root.join("release-components.json"),
            &json!({"schema": 1, "components": specs}).to_string(),
        );
        let editor = root.join("editor");
        write(&editor.join("engine/source.rs"), "engine source");
        write(&editor.join("sdk/source.rs"), "sdk source");
        write(
            &editor.join(".components-receipt.json"),
            &json!({"files": {"sdk/source.rs": sha256_bytes(b"sdk source")}}).to_string(),
        );
        let hk = root.join("games").join(HK);
        for (lock, name) in [("sdk.lock.json", "sdk"), ("emulator.lock.json", "emulator")] {
            write(
                &hk.join(lock),
                &json!({"repository": format!("https://github.com/owner/{name}.git"), "revision": specs[name]["revision"]})
                    .to_string(),
            );
        }
        let optional = OPTIONAL.iter().map(|(_, g)| *g);
        for game in GAMES.iter().copied().chain(optional) {
            let source = root.join("games").join(game);
            write(
                &source.join("components.lock.json"),
                &json!({"components": specs}).to_string(),
            );
            if OPTIONAL.iter().any(|(_, g)| *g == game) {
                continue;
            }
            let hydrated = source.join(".psoxide");
            write(&hydrated.join("engine/source.rs"), "engine source");
            write(&hydrated.join("sdk/source.rs"), "sdk source");
            write(
                &hydrated.join(".psoxide-source"),
                &format!("local:{}", editor.display()),
            );
        }
    }

    fn fake_git(dir: &Path, args: &[&str]) -> Result<String> {
        Ok(if args[0] == "ls-files" {
            "engine/source.rs".to_string()
        } else {
            dir.display().to_string()
        })
    }

    fn message<T>(result: Result<T>) -> String {
        match result {
            Ok(_) => panic!("expected a failure"),
            Err(error) => error.0,
        }
    }

    #[test]
    fn rejects_transitive_sdk_drift() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let specs = specs();
        write(
            &root.join("release-components.json"),
            &json!({"schema": 1, "components": specs}).to_string(),
        );
        for (owner, dependencies) in [
            ("editor", vec!["sdk", "emulator"]),
            ("emulator", vec!["sdk"]),
        ] {
            let nested: Map<String, Value> = dependencies
                .iter()
                .map(|n| (n.to_string(), specs[n].clone()))
                .collect();
            write(
                &root.join(owner).join("components.lock.json"),
                &json!({"components": nested}).to_string(),
            );
        }
        let revisions = specs.clone();
        let git_of = move |dir: &Path, args: &[&str]| -> Result<String> {
            if args[0] == "rev-parse" {
                let name = dir.file_name().unwrap().to_string_lossy().into_owned();
                Ok(revisions[&name]["revision"].as_str().unwrap().to_string())
            } else {
                Ok(String::new())
            }
        };
        let env = Env {
            root: root.clone(),
            git: &git_of,
        };
        let ran = RefCell::new(Vec::new());
        let bootstrap = |_: &Path, command: &[String]| -> Result<()> {
            ran.borrow_mut().push(command.to_vec());
            Ok(())
        };
        run_checks(&env, true, false, &bootstrap).unwrap();
        // Both importers are asked to verify, in order, before any build.
        assert_eq!(ran.borrow().len(), 2);
        assert!(ran
            .borrow()
            .iter()
            .all(|c| c.last().map(String::as_str) == Some("--check")));
        // The launcher now resolves SDK crates through the editor. Reject drift
        // in either importing owner before any build.
        for owner in ["editor", "emulator"] {
            let path = root.join(owner).join("components.lock.json");
            let original = fs::read_to_string(&path).unwrap();
            let mut lock: Value = serde_json::from_str(&original).unwrap();
            lock["components"]["sdk"]["revision"] = json!("d".repeat(40));
            fs::write(&path, lock.to_string()).unwrap();
            assert!(message(run_checks(&env, true, false, &bootstrap))
                .contains(&format!("{owner} sdk pin differs")));
            fs::write(&path, original).unwrap();
        }
    }

    #[test]
    fn requires_every_game_and_unchanged_imports() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fixture(&root);
        let env = Env {
            root: root.clone(),
            git: &fake_git,
        };
        verify_games(&env, false, false, false).unwrap();
        assert!(message(verify_games(&env, true, false, false)).contains("hl-psx: missing"));
        assert!(message(verify_games(&env, false, true, false)).contains("cs-psx: missing"));
        assert!(message(verify_games(&env, false, false, true))
            .contains("hk-psx: missing SDK hydration"));
        let state = root.join("games/hk-psx/.hkpsx/sdk.json");
        write(&state, &json!({"revision": "a".repeat(40)}).to_string());
        verify_games(&env, false, false, true).unwrap();
        write(&state, &json!({"revision": "d".repeat(40)}).to_string());
        assert!(message(verify_games(&env, false, false, true)).contains("hydrated SDK differs"));
        let source = root.join("games/voxide/.psoxide/sdk/source.rs");
        fs::write(&source, "tampered").unwrap();
        assert!(message(verify_games(&env, false, false, false)).contains("build input changed"));
        fs::write(&source, "sdk source").unwrap();
        fs::remove_file(root.join("games/voxide/.psoxide/.psoxide-source")).unwrap();
        assert!(message(verify_games(&env, false, false, false)).contains("missing current"));
    }

    #[test]
    fn rejects_standalone_component_drift() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fixture(&root);
        let path = root.join("games/nitroxide/components.lock.json");
        let mut lock: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        lock["components"]["sdk"]["revision"] = json!("d".repeat(40));
        fs::write(&path, lock.to_string()).unwrap();
        let env = Env {
            root,
            git: &fake_git,
        };
        assert!(message(verify_game_locks(&env)).contains("standalone sdk revision"));
    }

    #[test]
    fn rejects_hk_sdk_drift() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fixture(&root);
        let path = root.join("games/hk-psx/sdk.lock.json");
        let mut lock: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        lock["revision"] = json!("d".repeat(40));
        fs::write(&path, lock.to_string()).unwrap();
        let env = Env {
            root,
            git: &fake_git,
        };
        assert!(message(verify_game_locks(&env)).contains("hk-psx: sdk.lock.json revision"));
    }

    #[test]
    fn rejects_a_game_without_a_component_lock() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fixture(&root);
        fs::remove_file(root.join("games/pico8-psx/components.lock.json")).unwrap();
        let env = Env {
            root,
            git: &fake_git,
        };
        assert!(message(verify_game_locks(&env)).contains("pico8-psx: no components.lock.json"));
    }
}
