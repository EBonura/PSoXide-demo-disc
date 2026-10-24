//! One command that releases the public demo disc to itch.io.
//!
//!     disc-tools release --version vX.Y (--cue <cue> | --lineup <json> --frontend <frontend>)
//!         [--work <empty dir>] [--emulator-source <url or path>] [--chrome <path>]
//!         [--dry-run]
//!
//! In order: the disc repository must be clean, on `main` and level with
//! `origin/main`; a fresh clone of the emulator is checked out at the
//! `web_player` pin; the Public disc is reused (`--cue`) or pressed from a
//! lineup; `deploy-public` stages both packages into an empty directory;
//! `public-only` checks both folders together; `webdisc verify` replays the
//! compact payload against the download BIN; the browser journey suite from the
//! pinned emulator runs against the web folder. Then the summary is printed, the
//! operator types the version to confirm, and `butler push` runs for both
//! channels with `--userversion`, followed by `butler status` for both.
//!
//! `--dry-run` does everything except the pushes and the status calls: it
//! prints the exact commands instead, and reports repository-state problems as
//! warnings so a lane branch can rehearse the whole run. The packages are
//! hashed after the checks and again before the push, so nothing that changed
//! in between can be uploaded.

use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::args::Args;
use crate::deploy::{
    build_webdisc, public_image, run_program, valid_version, Tools, DOWNLOAD_TARGET,
    PLAYABLE_TARGET,
};
use crate::util::{
    git, read_json, repo_root, resolve_lenient, sha256_bytes, sha256_file, Error, Result,
};

/// Where the Public disc comes from.
enum DiscSource {
    /// An already pressed disc.
    Cue(PathBuf),
    /// Press it from a lineup file with `make lineup-disc`.
    Lineup { file: PathBuf, frontend: PathBuf },
}

/// Whether the pushes run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    DryRun,
    Publish,
}

struct Settings {
    version: String,
    disc: DiscSource,
    work: PathBuf,
    emulator_source: Option<String>,
    chrome: Option<PathBuf>,
    mode: Mode,
}

/// The facts printed in the summary, in the order they were established.
#[derive(Default)]
struct Summary {
    rows: Vec<(String, String)>,
    warnings: Vec<String>,
}

impl Summary {
    fn row(&mut self, key: &str, value: impl Into<String>) {
        self.rows.push((key.to_string(), value.into()));
    }
}

fn step(text: &str) {
    println!("\n==> {text}");
}

/// Quote a word for a shell command line, only when it needs it.
fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:=@+".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn shell_line(words: &[OsString]) -> String {
    words
        .iter()
        .map(|w| quote(&w.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The four butler invocations of a release: two pushes, then two status calls.
fn butler_commands(butler: &str, web: &Path, download: &Path, version: &str) -> Vec<Vec<OsString>> {
    let push = |directory: &Path, target: &str| -> Vec<OsString> {
        vec![
            butler.into(),
            "push".into(),
            directory.into(),
            target.into(),
            "--userversion".into(),
            version.into(),
        ]
    };
    let status =
        |target: &str| -> Vec<OsString> { vec![butler.into(), "status".into(), target.into()] };
    vec![
        push(download, DOWNLOAD_TARGET),
        push(web, PLAYABLE_TARGET),
        status(DOWNLOAD_TARGET),
        status(PLAYABLE_TARGET),
    ]
}

/// True when the operator typed exactly the version.
fn confirmed(input: &mut impl BufRead, version: &str) -> Result<bool> {
    let mut line = String::new();
    input.read_line(&mut line)?;
    Ok(line.trim() == version)
}

/// Problems that make the disc repository unfit to release from.
fn repo_problems(root: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    let branch = git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch != "main" {
        problems.push(format!("the repository is on {branch:?}, not main"));
    }
    if !git(root, &["status", "--porcelain", "--untracked-files=normal"])?.is_empty() {
        problems.push("the repository has uncommitted or untracked files".to_string());
    }
    git(root, &["fetch", "-q", "origin", "main"])?;
    let (head, upstream) = (
        git(root, &["rev-parse", "HEAD"])?,
        git(root, &["rev-parse", "FETCH_HEAD"])?,
    );
    if head != upstream {
        problems.push(format!(
            "HEAD {} is not origin/main {}",
            &head[..12],
            &upstream[..12]
        ));
    }
    Ok(problems)
}

/// The emulator revision and repository the browser player is built from.
fn web_player_pin(root: &Path) -> Result<(String, String)> {
    let lock = read_json(&root.join("release-components.json"))?;
    let selected = lock
        .get("web_player")
        .or_else(|| lock.get("components").and_then(|c| c.get("emulator")))
        .ok_or_else(|| Error("release-components.json names no web_player or emulator".into()))?;
    let field = |name: &str| {
        selected
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error(format!("the web player pin has no {name}")))
    };
    Ok((field("repository")?, field("revision")?))
}

/// A fresh checkout of `revision`, detached, in `destination`.
fn checkout_emulator(source: &str, revision: &str, destination: &Path) -> Result<()> {
    run_program(
        Command::new("git")
            .args(["clone", "-q", "--no-checkout", source])
            .arg(destination),
    )?;
    git(destination, &["checkout", "-q", "--detach", revision])?;
    let head = git(destination, &["rev-parse", "HEAD"])?;
    ensure!(head == revision, "checked out {head}, wanted {revision}");
    Ok(())
}

/// Press the Public disc from a lineup and return its cue.
fn press(root: &Path, version: &str, file: &Path, frontend: &Path, work: &Path) -> Result<PathBuf> {
    let dist = work.join("press");
    fs::create_dir_all(&dist)?;
    let name = format!("PSoXide Demo Disc {version}");
    run_program(
        Command::new("make")
            .arg("lineup-disc")
            .arg(format!("LINEUP={}", file.display()))
            .arg(format!("DISC_NAME={name}"))
            .arg(format!("DISC_VERSION={version}"))
            .arg(format!("DIST={}", dist.display()))
            .arg(format!("FRONTEND={}", frontend.display()))
            .current_dir(root),
    )?;
    Ok(dist.join(format!("{name}.cue")))
}

/// A digest of every file under `directory`: paths, sizes and contents.
fn tree_digest(directory: &Path) -> Result<String> {
    let mut lines = Vec::new();
    for (relative, path) in crate::deploy::files_under(directory)? {
        if path.is_file() {
            lines.push(format!("{} {}", relative.display(), sha256_file(&path)?));
        }
    }
    Ok(sha256_bytes(lines.join("\n").as_bytes()))
}

fn total_size(directory: &Path) -> Result<(usize, u64)> {
    let mut count = 0;
    let mut bytes = 0;
    for (_, path) in crate::deploy::files_under(directory)? {
        if path.is_file() {
            count += 1;
            bytes += fs::metadata(&path)?.len();
        }
    }
    Ok((count, bytes))
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn parse(args: &[String]) -> Result<Settings> {
    let args = Args::parse(
        args,
        &[
            "version",
            "cue",
            "lineup",
            "frontend",
            "work",
            "emulator-source",
            "chrome",
        ],
        &["dry-run"],
    )?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let version = args.require("version")?.to_string();
    ensure!(
        valid_version(&version)?,
        "provide a release version such as v0.34"
    );
    let disc = match (args.path("cue"), args.path("lineup")) {
        (Some(cue), None) => DiscSource::Cue(cue),
        (None, Some(file)) => DiscSource::Lineup {
            file,
            frontend: args.require_path("frontend")?,
        },
        _ => bail!("give exactly one of --cue (reuse a pressed disc) or --lineup (press one)"),
    };
    let work = args.path("work").unwrap_or_else(|| {
        repo_root()
            .map_or_else(|_| PathBuf::from("."), |r| r.join("build"))
            .join(format!("release-{version}"))
    });
    Ok(Settings {
        version,
        disc,
        work,
        emulator_source: args.get("emulator-source").map(str::to_string),
        chrome: args.path("chrome"),
        mode: if args.has("dry-run") {
            Mode::DryRun
        } else {
            Mode::Publish
        },
    })
}

/// Run the whole release; see the module documentation.
pub fn run(args: &[String]) -> Result<i32> {
    let settings = parse(args)?;
    let root = repo_root()?.canonicalize()?;
    let work = resolve_lenient(&settings.work);
    let mut summary = Summary::default();
    let dry = settings.mode == Mode::DryRun;
    let butler = "butler";
    summary.row("version", &settings.version);

    step("disc repository is clean, on main and current");
    let problems = repo_problems(&root)?;
    if problems.is_empty() {
        println!("ok");
    } else if dry {
        for problem in &problems {
            println!("warning (fatal without --dry-run): {problem}");
            summary.warnings.push(problem.clone());
        }
    } else {
        bail!("not releasable: {}", problems.join("; "));
    }
    summary.row("disc repository", git(&root, &["rev-parse", "HEAD"])?);

    ensure!(
        !work.exists() || fs::read_dir(&work)?.next().is_none(),
        "{} must be empty or absent: a release starts from scratch",
        work.display()
    );
    fs::create_dir_all(&work)?;

    step("butler is installed");
    run_program(Command::new(butler).arg("--version"))?;

    step("emulator at the web_player pin");
    let (repository, revision) = web_player_pin(&root)?;
    let source = settings
        .emulator_source
        .clone()
        .unwrap_or_else(|| format!("https://github.com/{repository}.git"));
    let emulator = work.join("emulator");
    checkout_emulator(&source, &revision, &emulator)?;
    println!("{} at {revision}", emulator.display());
    summary.row("emulator (web player)", &revision);

    step("Public disc");
    let cue = match &settings.disc {
        DiscSource::Cue(cue) => resolve_lenient(cue),
        DiscSource::Lineup { file, frontend } => {
            press(&root, &settings.version, file, frontend, &work)?
        }
    };
    let image = public_image(&cue)?;
    let disc_sha = sha256_file(&image)?;
    println!("{} sha256 {disc_sha}", cue.display());
    summary.row("disc image", disc_sha);

    step("deploy-public (stage both packages, no upload)");
    let out = work.join("out");
    let path_text = |path: &Path| path.display().to_string();
    let deploy_args = vec![
        "--cue".to_string(),
        path_text(&cue),
        "--version".to_string(),
        settings.version.clone(),
        "--emulator".to_string(),
        path_text(&emulator),
        "--out".to_string(),
        path_text(&out),
    ];
    crate::deploy::run(&deploy_args)?;
    let (web, download) = (out.join("web"), out.join("download"));

    step("public-only on both folders");
    let both = [web.display().to_string(), download.display().to_string()];
    ensure!(
        crate::publiconly::run(&both)? == 0,
        "public-only found a private trace"
    );
    summary.row("public-only", "clean on web and download");

    step("webdisc verify (compact payload against the download BIN)");
    let webdisc = build_webdisc(&emulator, &Tools::real())?;
    run_program(
        Command::new(&webdisc)
            .arg("verify")
            .arg(&web)
            .arg(crate::web::LAZY_STEM)
            .arg(&image),
    )?;
    summary.row(
        "webdisc verify",
        "every sector identical to the download BIN",
    );

    step("browser journey suite against the web folder");
    let suite = emulator.join("tools/web-journeys/run.mjs");
    ensure!(
        suite.is_file(),
        "the pinned emulator has no tools/web-journeys/run.mjs; pin a newer web_player"
    );
    let report = out.join("journeys.md");
    let mut node = Command::new("node");
    node.arg(&suite)
        .arg("--site")
        .arg(&web)
        .arg("--report")
        .arg(&report);
    if let Some(chrome) = &settings.chrome {
        node.arg("--chrome").arg(chrome);
    }
    run_program(&mut node)?;
    summary.row(
        "journeys",
        format!("all passed, report {}", report.display()),
    );

    let (digest_web, digest_download) = (tree_digest(&web)?, tree_digest(&download)?);
    let (web_files, web_bytes) = total_size(&web)?;
    let (download_files, download_bytes) = total_size(&download)?;
    summary.row(
        "web package",
        format!("{web_files} files, {}", megabytes(web_bytes)),
    );
    summary.row(
        "download package",
        format!("{download_files} files, {}", megabytes(download_bytes)),
    );
    summary.row("packages", out.display().to_string());

    println!("\n==== release summary ====");
    for (key, value) in &summary.rows {
        println!("{key:<24} {value}");
    }
    for warning in &summary.warnings {
        println!("warning                  {warning}");
    }
    let commands = butler_commands(butler, &web, &download, &settings.version);
    if dry {
        println!("\nDRY RUN: nothing was pushed. A real run would execute:");
        for command in &commands {
            println!("  {}", shell_line(command));
        }
        return Ok(0);
    }

    print!(
        "\nType {} to push both channels, anything else cancels: ",
        settings.version
    );
    io::stdout().flush()?;
    if !confirmed(&mut io::stdin().lock(), &settings.version)? {
        println!("cancelled; nothing was pushed");
        return Ok(1);
    }
    ensure!(
        tree_digest(&web)? == digest_web && tree_digest(&download)? == digest_download,
        "the staged packages changed after they were checked; nothing was pushed"
    );
    for command in &commands {
        println!("\n$ {}", shell_line(command));
        let (program, rest) = command
            .split_first()
            .expect("a butler command is never empty");
        run_program(Command::new(program).args(rest))?;
    }
    println!("\nreleased {}", settings.version);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn confirmation_needs_the_exact_version() {
        assert!(confirmed(&mut Cursor::new("v0.48\n"), "v0.48").unwrap());
        assert!(confirmed(&mut Cursor::new("  v0.48  \n"), "v0.48").unwrap());
        assert!(!confirmed(&mut Cursor::new("v0.4\n"), "v0.48").unwrap());
        assert!(!confirmed(&mut Cursor::new("yes\n"), "v0.48").unwrap());
        assert!(!confirmed(&mut Cursor::new(""), "v0.48").unwrap());
    }

    #[test]
    fn butler_pushes_both_channels_then_checks_both() {
        let commands = butler_commands(
            "butler",
            Path::new("/o/web"),
            Path::new("/o/my download"),
            "v0.48",
        );
        let lines: Vec<String> = commands.iter().map(|c| shell_line(c)).collect();
        assert_eq!(
            lines,
            [
                "butler push '/o/my download' bonnie-studios/psoxide-demo-disc:psx --userversion v0.48",
                "butler push /o/web bonnie-studios/psoxide:html5 --userversion v0.48",
                "butler status bonnie-studios/psoxide-demo-disc:psx",
                "butler status bonnie-studios/psoxide:html5",
            ]
        );
    }

    #[test]
    fn digest_sees_a_changed_or_added_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a"), "one").unwrap();
        let before = tree_digest(dir.path()).unwrap();
        assert_eq!(before, tree_digest(dir.path()).unwrap());
        fs::write(dir.path().join("a"), "two").unwrap();
        let changed = tree_digest(dir.path()).unwrap();
        assert_ne!(before, changed);
        fs::write(dir.path().join("b"), "x").unwrap();
        assert_ne!(changed, tree_digest(dir.path()).unwrap());
    }

    #[test]
    fn arguments_need_a_version_and_one_disc_source() {
        let raw = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(parse(&raw(&["--cue", "a.cue"])).is_err());
        assert!(parse(&raw(&["--version", "0.48", "--cue", "a.cue"])).is_err());
        assert!(parse(&raw(&["--version", "v0.48"])).is_err());
        assert!(parse(&raw(&["--version", "v0.48", "--cue", "a", "--lineup", "b"])).is_err());
        assert!(parse(&raw(&["--version", "v0.48", "--lineup", "b"])).is_err());
        let ok = parse(&raw(&["--version", "v0.48", "--cue", "a.cue", "--dry-run"])).unwrap();
        assert!(ok.mode == Mode::DryRun);
    }

    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn repo_problems_name_branch_dirt_and_drift() {
        let base = tempfile::tempdir().unwrap();
        let (origin, clone) = (base.path().join("origin"), base.path().join("clone"));
        fs::create_dir(&origin).unwrap();
        git_in(&origin, &["init", "-q", "-b", "main"]);
        fs::write(origin.join("f"), "1").unwrap();
        git_in(&origin, &["add", "-A"]);
        git_in(&origin, &["commit", "-q", "-m", "one"]);
        git_in(
            base.path(),
            &["clone", "-q", origin.to_str().unwrap(), "clone"],
        );
        assert!(repo_problems(&clone).unwrap().is_empty());

        fs::write(clone.join("untracked"), "x").unwrap();
        assert_eq!(repo_problems(&clone).unwrap().len(), 1);
        fs::remove_file(clone.join("untracked")).unwrap();

        git_in(&clone, &["checkout", "-q", "-b", "lane"]);
        assert_eq!(repo_problems(&clone).unwrap().len(), 1);
        git_in(&clone, &["checkout", "-q", "main"]);

        fs::write(origin.join("f"), "2").unwrap();
        git_in(&origin, &["commit", "-q", "-am", "two"]);
        assert_eq!(repo_problems(&clone).unwrap().len(), 1);
    }
}
