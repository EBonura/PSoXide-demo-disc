//! Verify and bootstrap the exact repositories selected for this demo disc.
//!
//! `release-components.json` pins the SDK, editor and emulator revisions. This
//! checks that the submodule checkouts are those revisions and clean, that the
//! pins nested inside the editor and emulator agree with the disc's. (The checks
//! of the standalone games' own locks and hydrated trees belonged to the retired
//! non-lineup `make disc` path and are gone with it.)
//!
//! The bootstrap itself, materialising the pinned sources where Cargo expects
//! them, belongs to the editor and emulator repositories, and they have shipped
//! it two ways. An older pin carries `tools/bootstrap-components.py`; a newer
//! one drops the script and hydrates with the SDK's `psoxide-components`, built
//! once per locked SDK revision into the tree's `.tools/` (what its `make
//! bootstrap` does). [`bootstrap_tree`] runs whichever the pinned tree has.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

use crate::args::Args;
use crate::disc::image_for_cue;
use crate::util::{
    command_output, file_record, git, read_json, repo_root, write_receipt, Error,
    Result,
};

type GitFn<'a> = &'a dyn Fn(&Path, &[&str]) -> Result<String>;
/// Bootstrap one pinned tree: the tree, then the flags to give its tool.
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

/// True when the tree's `components.lock.json` marks its SDK as vendored.
fn vendored_sdk(tree: &Path) -> bool {
    read_json(&tree.join("components.lock.json"))
        .map(|lock| lock["components"]["sdk"]["vendored"].as_bool() == Some(true))
        .unwrap_or(false)
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
        // An emulator that vendors its SDK carries the crates in git: it has no
        // SDK pin to agree with the disc's and nothing to bootstrap.
        if nested["components"]["sdk"]["vendored"].as_bool() == Some(true) {
            continue;
        }
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
        if name == "emulator" && vendored_sdk(&source) {
            continue;
        }
        let mut flags: Vec<String> = Vec::new();
        if check || check_main {
            flags.push("--check".into());
        } else {
            flags.push("--source".into());
            flags.push(format!(
                "sdk={}",
                env.root.join(field(&specs["sdk"], "path")?).display()
            ));
            flags.push("--source".into());
            flags.push(format!(
                "emulator={}",
                env.root.join(field(&specs["emulator"], "path")?).display()
            ));
        }
        bootstrap(&source, &flags)?;
    }
    println!("Release component revisions and imported source receipts verified");
    Ok(())
}

/// Where the SDK's `psoxide-link` package is installed from. This is the
/// repository the editor and emulator Makefiles install it from too.
const SDK_GIT: &str = "https://github.com/EBonura/PSoXide";

/// Bootstrap one pinned tree with the tool that tree ships.
///
/// A tree that still has `tools/bootstrap-components.py` (an older pin) is run
/// exactly as before, with `python`. A tree without it (a newer pin) is
/// hydrated with `psoxide-components`, the same flags and the tree as `--root`.
pub fn bootstrap_tree(python: &OsStr, tree: &Path, flags: &[String]) -> Result<()> {
    let script = tree.join("tools/bootstrap-components.py");
    let (program, arguments): (PathBuf, Vec<String>) = if script.is_file() {
        let mut arguments = vec![script.display().to_string()];
        arguments.extend(flags.iter().cloned());
        (PathBuf::from(python), arguments)
    } else {
        let mut arguments = vec!["--root".to_string(), tree.display().to_string()];
        arguments.extend(flags.iter().cloned());
        (components_binary(tree)?, arguments)
    };
    let status = Command::new(&program)
        .args(&arguments)
        .status()
        .map_err(|e| Error(format!("cannot run {}: {e}", program.display())))?;
    ensure!(
        status.success(),
        "{} {} failed ({status})",
        program.display(),
        arguments.join(" ")
    );
    Ok(())
}

/// The tree's `psoxide-components`, installed from the SDK revision its own
/// `components.lock.json` pins unless a previous bootstrap already did. The
/// location is the one its Makefile uses, so `make bootstrap` and this share it.
fn components_binary(tree: &Path) -> Result<PathBuf> {
    let lock = read_json(&tree.join("components.lock.json"))?;
    let revision = lock["components"]["sdk"]["revision"]
        .as_str()
        .ok_or_else(|| {
            Error(format!(
                "{}: no sdk revision in components.lock.json",
                tree.display()
            ))
        })?;
    let root = tree.join(".tools").join(format!("sdk-{revision}"));
    let binary = root.join("bin/psoxide-components");
    if !binary.is_file() {
        let status = Command::new("cargo")
            .args([
                "install", "--locked", "--git", SDK_GIT, "--rev", revision, "--root",
            ])
            .arg(&root)
            .arg("psoxide-link")
            .current_dir(tree)
            .status()
            .map_err(|e| Error(format!("cannot run cargo: {e}")))?;
        ensure!(
            status.success() && binary.is_file(),
            "cargo install of psoxide-link at {revision} failed ({status})"
        );
    }
    Ok(binary)
}

/// Run a sub-repository's bootstrap with the interpreter on PATH (`PYTHON`
/// overrides it) when the tree still has the Python script.
fn run_bootstrap(tree: &Path, flags: &[String]) -> Result<()> {
    let python = std::env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    bootstrap_tree(&python, tree, flags)
}

/// The component receipt for a pressed lineup: every submodule at its
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
    use crate::util::sha256_bytes;
    use std::fs;
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

    /// The release tuple and the editor tree the checks look at, all consistent.
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
    fn a_vendored_emulator_has_no_sdk_pin_to_agree_or_bootstrap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let specs = specs();
        write(
            &root.join("release-components.json"),
            &json!({"schema": 1, "components": specs}).to_string(),
        );
        write(
            &root.join("editor").join("components.lock.json"),
            &json!({"components": {"sdk": specs["sdk"].clone(), "emulator": specs["emulator"].clone()}})
                .to_string(),
        );
        write(
            &root.join("emulator").join("components.lock.json"),
            &json!({"components": {"sdk": {"revision": "d".repeat(40), "vendored": true}}})
                .to_string(),
        );
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
            root,
            git: &git_of,
        };
        let ran = RefCell::new(Vec::new());
        let bootstrap = |tree: &Path, _: &[String]| -> Result<()> {
            ran.borrow_mut().push(tree.to_path_buf());
            Ok(())
        };
        run_checks(&env, true, false, &bootstrap).unwrap();
        let ran = ran.borrow();
        assert_eq!(ran.len(), 1);
        assert!(ran[0].ends_with("editor"));
    }

    /// A tree that logs the arguments its bootstrap tool was given.
    fn logging_tool(path: &Path, log: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            path,
            format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn an_old_pin_runs_its_python_script() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("emulator");
        let log = dir.path().join("log");
        logging_tool(&tree.join("tools/bootstrap-components.py"), &log);
        // A psoxide-components in .tools must not be preferred over the script.
        fs::write(
            tree.join("components.lock.json"),
            json!({"components": {"sdk": {"revision": "a".repeat(40)}}}).to_string(),
        )
        .unwrap();
        logging_tool(
            &tree.join(format!(
                ".tools/sdk-{}/bin/psoxide-components",
                "a".repeat(40)
            )),
            &dir.path().join("wrong"),
        );
        // The "interpreter" is `env`, which runs the script as a program.
        bootstrap_tree(OsStr::new("env"), &tree, &["--check".into()]).unwrap();
        assert_eq!(fs::read_to_string(&log).unwrap().trim(), "--check");
        assert!(!dir.path().join("wrong").exists());
    }

    #[test]
    fn a_new_pin_runs_the_sdk_components_tool_on_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let tree = dir.path().join("emulator");
        let log = dir.path().join("log");
        let revision = "b".repeat(40);
        fs::create_dir_all(&tree).unwrap();
        fs::write(
            tree.join("components.lock.json"),
            json!({"components": {"sdk": {"revision": revision}}}).to_string(),
        )
        .unwrap();
        logging_tool(
            &tree.join(format!(".tools/sdk-{revision}/bin/psoxide-components")),
            &log,
        );
        let flags = vec!["--source".to_string(), "sdk=/somewhere".to_string()];
        bootstrap_tree(OsStr::new("no-such-python"), &tree, &flags).unwrap();
        assert_eq!(
            fs::read_to_string(&log).unwrap().trim(),
            format!("--root {} --source sdk=/somewhere", tree.display())
        );
        // A failing tool is an error, whichever tool it is.
        fs::write(
            tree.join(format!(".tools/sdk-{revision}/bin/psoxide-components")),
            "#!/bin/sh\nexit 3\n",
        )
        .unwrap();
        assert!(message(bootstrap_tree(OsStr::new("x"), &tree, &[])).contains("failed"));
    }
}
