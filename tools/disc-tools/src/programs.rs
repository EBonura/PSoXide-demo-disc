//! Boot the demo disc's independent programs through the carousel, headless,
//! and check each one's live output: the launcher booted and chain-loaded the
//! right number of times, the program printed its marker, the final PC is in
//! RAM and the last frame is not blank.
//!
//! The route for each program is derived from the pressed table, so adding or
//! reordering carousel cards cannot silently point a route at the wrong game.
//! The Arcade guests are reached through the Arcade collection's own menu.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::args::Args;
use crate::disc::{self, FLAG_HIDDEN};
use crate::replay::{
    number, parse_int, pattern, run_merged, shlex_join, shlex_split, toc_rows, Csv, Row,
};
use crate::util::{Error, Result};

pub(crate) const FAILURE_MARKERS: [&str; 4] = [
    "PANIC:",
    "psx_rt::halt",
    "STACK/DATA COLLISION",
    "payload checksum mismatch",
];

static TICK_SUMMARY: OnceLock<regex::Regex> = OnceLock::new();
static DISPLAY_SUMMARY: OnceLock<regex::Regex> = OnceLock::new();

/// One program's route through the carousel and what it has to print.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    name: &'static str,
    presses: String,
    steps: u64,
    launcher_count: usize,
    markers: Vec<&'static str>,
    cdda_owner: Option<&'static str>,
}

/// The BIN a cue names: the first `FILE` line, shell-split as Python did.
pub(crate) fn cue_bin(cue: &Path) -> Result<PathBuf> {
    let bytes = fs::read(cue).map_err(|e| Error(format!("{}: {e}", cue.display())))?;
    ensure!(bytes.is_ascii(), "{}: cue is not ASCII", cue.display());
    let text = String::from_utf8_lossy(&bytes);
    for line in text.lines() {
        let words = shlex_split(line)?;
        if words.len() >= 2 && words[0].eq_ignore_ascii_case("FILE") {
            let image = Path::new(&words[1]);
            return Ok(if image.is_absolute() {
                image.to_path_buf()
            } else {
                cue.parent().unwrap_or(Path::new(".")).join(image)
            });
        }
    }
    bail!("{}: no FILE line", cue.display())
}

pub(crate) fn read_rows(image: &Path) -> Result<Vec<Row>> {
    toc_rows(&disc::read_user_sectors(
        image,
        disc::TOC_LBA,
        disc::TOC_SECTORS,
    )?)
}

/// The presses that bring `target` to the middle of the carousel and launch
/// it, and the tick of the launching CROSS.
pub(crate) fn menu_presses(entries: &[Row], target: &str) -> Result<(Vec<String>, i64)> {
    let visible: Vec<&str> = entries
        .iter()
        .filter(|e| e.flags & FLAG_HIDDEN == 0)
        .map(|e| e.name.as_str())
        .collect();
    let Some(index) = visible.iter().position(|name| *name == target) else {
        bail!("{target:?} is not visible: {}", visible.join(", "));
    };
    let count = visible.len() as i64 + 1; // Credits is the final non-program card.
    let index = index as i64;
    let (left, right) = (index, count - index);
    let (button, moves) = if left <= right {
        ("left", left)
    } else {
        ("right", right)
    };
    let mut presses: Vec<String> = (0..moves)
        .map(|step| format!("{}:{button}:8", 400 + step * 200))
        .collect();
    // Let the final backdrop decode and carousel movement finish before launch.
    // Cards reached after several rapid moves can still be selected while X is
    // temporarily ignored during that visual transition.
    let cross = if moves != 0 {
        1_000.max(400 + (moves - 1) * 200 + 600)
    } else {
        1_000
    };
    presses.push(format!("{cross}:cross:12"));
    Ok((presses, cross))
}

fn extended(base: &[String], extra: &[String]) -> Vec<String> {
    base.iter().chain(extra).cloned().collect()
}

fn routes(entries: &[Row]) -> Result<Vec<Route>> {
    let outer = |name: &str| menu_presses(entries, name);
    let (voxide, _) = outer("VOXIDE")?;
    let (nitroxide, _) = outer("NITROXIDE")?;
    let (celeste, celeste_cross) = outer("CELESTE COLLECTION")?;
    let (psxcel, _) = outer("PSXCEL")?;
    let (arcade, arcade_cross) = outer("PSOXIDE ARCADE")?;
    let inner_ready = arcade_cross + 600;
    let at = |tick: i64, button: &str, hold: u32| format!("{tick}:{button}:{hold}");

    let breakout = extended(
        &arcade,
        &[
            at(inner_ready, "cross", 12),
            at(inner_ready + 500, "cross", 12),
        ],
    );
    let invaders = extended(
        &arcade,
        &[
            at(inner_ready, "right", 8),
            at(inner_ready + 200, "cross", 12),
            at(inner_ready + 700, "cross", 12),
        ],
    );
    let magikarp = extended(
        &arcade,
        &[
            at(inner_ready, "right", 8),
            at(inner_ready + 200, "right", 8),
            at(inner_ready + 400, "cross", 12),
            at(inner_ready + 900, "cross", 12),
        ],
    );
    let classic = extended(
        &celeste,
        &[
            at(celeste_cross + 1600, "cross", 12),
            at(celeste_cross + 2200, "cross", 12),
            at(celeste_cross + 2800, "cross", 12),
        ],
    );
    let classic2 = extended(
        &celeste,
        &[
            at(celeste_cross + 1500, "right", 8),
            at(celeste_cross + 1700, "cross", 12),
            at(celeste_cross + 2300, "cross", 12),
            at(celeste_cross + 2900, "cross", 12),
        ],
    );

    let route = |name, presses: &[String], steps, launcher_count, markers: &[&'static str]| Route {
        name,
        presses: presses.join(","),
        steps,
        launcher_count,
        markers: markers.to_vec(),
        cdda_owner: None,
    };
    Ok(vec![
        route("voxide", &voxide, 450_000_000, 1, &["voxide: boot"]),
        // NitroXide and Celeste sit mid-carousel on the 13-card pressing,
        // six moves from either end, so the launch press lands at route tick
        // 2000; 450M steps stopped 19 ticks later, before the chain-load.
        route(
            "nitroxide",
            &nitroxide,
            700_000_000,
            1,
            &["psx-engine: loading ready"],
        ),
        route("celeste", &celeste, 700_000_000, 1, &[]),
        // Wait for the collection intro, choose each cart, then dismiss its
        // title screen. The private pressing adds another outer carousel move;
        // leave time for the cart title fade and retry Cross once after it.
        // These exercise both linked games, not just the collection menu.
        route("celeste-classic", &classic, 1_100_000_000, 1, &[]),
        route("celeste-classic-2", &classic2, 1_100_000_000, 1, &[]),
        // PSXcel is six carousel moves from the initial card. Give the launcher
        // enough emulated time to reach the delayed launch press before judging
        // the guest, rather than stopping while its card is merely selected.
        route("psxcel", &psxcel, 450_000_000, 1, &[]),
        route("arcade-breakout", &breakout, 700_000_000, 2, &[]),
        route(
            "arcade-invaders",
            &invaders,
            700_000_000,
            2,
            &["invaders: init ok"],
        ),
        Route {
            cdda_owner: Some("PSOXIDE ARCADE"),
            ..route(
                "arcade-magikarp",
                &magikarp,
                700_000_000,
                2,
                &["magikarp: cdda ok"],
            )
        },
    ])
}

/// The last frame has to show something: a few colours, some bright pixels,
/// a dark floor and real contrast. Sampled every 17th pixel.
pub(crate) fn require_live_ppm(path: &Path) -> Result<()> {
    let data = fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let malformed = || Error(format!("{}: malformed PPM", path.display()));
    let mut parts = data.splitn(4, |&b| b == b'\n');
    let (Some(magic), Some(dimensions), Some(maximum), Some(pixels)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(malformed());
    };
    let dimensions: Vec<&[u8]> = dimensions
        .split(|b| b.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect();
    let [width, height] = dimensions[..] else {
        return Err(malformed());
    };
    let parse = |word: &[u8]| -> Result<usize> {
        std::str::from_utf8(word)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(malformed)
    };
    let (width, height) = (parse(width)?, parse(height)?);
    ensure!(
        magic == b"P6" && maximum == b"255" && (width, height) == (320, 240),
        "{}: expected a 320x240 P6 display",
        path.display()
    );
    ensure!(
        pixels.len() == width * height * 3,
        "{}: pixel payload has the wrong size",
        path.display()
    );
    let mut colours = std::collections::HashSet::new();
    let mut bright = 0;
    let mut levels: Vec<u8> = Vec::new();
    for at in (0..pixels.len()).step_by(3 * 17) {
        let Some(colour) = pixels.get(at..at + 3) else {
            continue;
        };
        colours.insert([colour[0], colour[1], colour[2]]);
        let level = *colour.iter().max().expect("three channels");
        levels.push(level);
        bright += usize::from(level > 40);
    }
    // The software display expands PS1 limited-range black to RGB 16, not
    // zero. Flat menu screens also have a deliberately tiny palette, so use
    // contrast and variation instead of demanding photographic colour counts.
    let lowest = levels.iter().copied().min().unwrap_or(255);
    let highest = levels.iter().copied().max().unwrap_or(0);
    ensure!(
        !(colours.len() < 4 || bright < 20 || lowest > 24 || highest.saturating_sub(lowest) < 40),
        "{}: display is blank or visually implausible",
        path.display()
    );
    Ok(())
}

/// The CD log has to show a Play (0x03) of the relocated track.
fn require_track(cd_log: &Path, track: u32) -> Result<()> {
    let expected = format!("{track:02}");
    let csv = Csv::read(cd_log)?;
    for row in &csv.rows {
        if csv.cell(row, "command")? == Some("0x03")
            && csv.cell(row, "param_len")? == Some("1")
            && csv
                .cell(row, "params")?
                .is_some_and(|params| params.trim() == expected)
        {
            return Ok(());
        }
    }
    bail!(
        "{}: no Play command for relocated track {expected}",
        cd_log.display()
    )
}

fn run_route(
    frontend: &Path,
    cue: &Path,
    output: &Path,
    route: &Route,
    entries: &[Row],
) -> Result<String> {
    let run_dir = output.join(route.name);
    fs::create_dir_all(&run_dir)?;
    let display = run_dir.join("final.ppm");
    let cd_log = run_dir.join("cd.csv");
    let arguments: Vec<String> = vec![
        "launch".into(),
        "--embedded-playtest".into(),
        "--config-dir".into(),
        run_dir.join("config").display().to_string(),
        "--path".into(),
        cue.display().to_string(),
        "--steps".into(),
        route.steps.to_string(),
        "--press".into(),
        route.presses.clone(),
        "--guest-debug-log".into(),
        "--cd-command-log".into(),
        cd_log.display().to_string(),
        "--dump-display".into(),
        display.display().to_string(),
        "--dump-hash".into(),
    ];
    let mut command_words = vec![frontend.display().to_string()];
    command_words.extend(arguments.iter().cloned());
    fs::write(
        run_dir.join("command.txt"),
        format!("{}\n", shlex_join(&command_words)),
    )?;
    let (code, stdout) = run_merged(
        Command::new(frontend).args(&arguments),
        &run_dir.join("stdout.txt"),
        None,
    )?;
    let name = route.name;
    ensure!(code == 0, "{name}: frontend exited {code}\n{stdout}");
    for marker in FAILURE_MARKERS {
        ensure!(
            !stdout.contains(marker),
            "{name}: output contains failure marker {marker:?}"
        );
    }
    ensure!(
        stdout.matches("launcher: booted").count() == route.launcher_count,
        "{name}: wrong launcher boot count"
    );
    ensure!(
        stdout.matches("launcher: chain-loading").count() == route.launcher_count,
        "{name}: wrong chain-load count"
    );
    for marker in &route.markers {
        ensure!(
            stdout.contains(marker),
            "{name}: missing runtime marker {marker:?}"
        );
    }
    let tick = pattern(
        &TICK_SUMMARY,
        r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)",
    )
    .captures(&stdout);
    let summary = pattern(
        &DISPLAY_SUMMARY,
        r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)",
    )
    .captures(&stdout);
    let tick = match tick {
        Some(tick) if number(&tick[1], 10)? == route.steps => tick,
        _ => bail!("{name}: did not retire the requested instruction count"),
    };
    let pc = parse_int(&tick[3], 16)?;
    ensure!(
        (0x8001_0000..0x8020_0000).contains(&pc),
        "{name}: final PC {pc:#010x} is outside PlayStation RAM"
    );
    let summary = match summary {
        Some(summary) if (number(&summary[2], 10)?, number(&summary[3], 10)?) == (320, 240) => {
            summary
        }
        _ => bail!("{name}: missing 320x240 display summary"),
    };
    require_live_ppm(&display)?;
    if let Some(owner) = route.cdda_owner {
        let Some(owner) = entries.iter().find(|row| row.name == owner) else {
            bail!("{name}: no {owner} entry in the disc table");
        };
        require_track(&cd_log, owner.cdda_track_base + 2)?;
    }
    Ok(format!("{name}: PASS {} pc={pc:#010x}", &summary[1]))
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(raw, &["frontend", "cue", "out", "jobs", "only"], &[])?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let frontend = args.require_path("frontend")?;
    let cue = args.require_path("cue")?;
    let out = args.require_path("out")?;
    let jobs = args.int("jobs")?.unwrap_or(1).max(1) as usize;

    let entries = read_rows(&cue_bin(&cue)?)?;
    let mut selected = routes(&entries)?;
    let wanted = args.all("only");
    if !wanted.is_empty() {
        selected.retain(|route| wanted.contains(&route.name));
        let mut missing: Vec<&str> = wanted
            .iter()
            .copied()
            .filter(|name| !selected.iter().any(|route| route.name == *name))
            .collect();
        missing.sort_unstable();
        missing.dedup();
        ensure!(
            missing.is_empty(),
            "unknown route(s): {}",
            missing.join(", ")
        );
    }

    fs::create_dir_all(&out)?;
    let next = AtomicUsize::new(0);
    let failures = Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(selected.len().max(1)) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some(route) = selected.get(index) else {
                    break;
                };
                // Report every failed route in one pass.
                match run_route(&frontend, &cue, &out, route, &entries) {
                    Ok(line) => println!("{line}"),
                    Err(error) => {
                        let line = format!("{}: FAIL {error}", route.name);
                        eprintln!("{line}");
                        failures.lock().expect("failure list").push(line);
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().expect("failure list");
    if !failures.is_empty() {
        eprintln!("program headless check: {} failure(s)", failures.len());
        return Ok(1);
    }
    println!("program headless check: PASS ({} routes)", selected.len());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(names: &[&str], hidden_first: bool) -> Vec<u8> {
        let mut toc = vec![0u8; disc::TOC_SECTORS as usize * disc::USER_DATA_BYTES];
        toc[..8].copy_from_slice(disc::TOC_MAGIC);
        toc[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
        for (index, name) in names.iter().enumerate() {
            let at = disc::TOC_HEADER_BYTES + index * disc::TOC_ENTRY_BYTES;
            toc[at..at + name.len()].copy_from_slice(name.as_bytes());
            let base = at + disc::TOC_NAME_BYTES + 8;
            toc[base..base + 4].copy_from_slice(&(index as u32 + 4).to_le_bytes());
        }
        if hidden_first {
            let at = disc::TOC_HEADER_BYTES + disc::TOC_FLAGS_AT;
            toc[at..at + 4].copy_from_slice(&FLAG_HIDDEN.to_le_bytes());
        }
        toc
    }

    #[test]
    fn parse_entries_and_menu_route() {
        let toc = table(&["CORTEX", "VOXIDE", "NITROXIDE"], true);
        let entries = toc_rows(&toc).unwrap();
        assert!(entries[0].hidden());
        assert_eq!(entries[2].cdda_track_base, 6);
        let (presses, cross) = menu_presses(&entries, "NITROXIDE").unwrap();
        assert_eq!(presses, ["400:left:8", "1000:cross:12"]);
        assert_eq!(cross, 1000);
    }

    #[test]
    fn menu_route_rejects_hidden_and_unknown_programs() {
        let entries = toc_rows(&table(&["CORTEX", "VOXIDE"], true)).unwrap();
        assert!(menu_presses(&entries, "CORTEX").is_err());
        assert!(menu_presses(&entries, "NOPE").is_err());
        assert!(toc_rows(&[0u8; 8192]).is_err());
    }

    fn write(dir: &Path, name: &str, pixels: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut data = b"P6\n320 240\n255\n".to_vec();
        data.extend_from_slice(pixels);
        fs::write(&path, data).unwrap();
        path
    }

    #[test]
    fn live_ppm_rejects_blank_frame() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "blank.ppm", &vec![0u8; 320 * 240 * 3]);
        let error = require_live_ppm(&path).unwrap_err().to_string();
        assert!(error.contains("blank or visually implausible"), "{error}");
    }

    #[test]
    fn live_ppm_accepts_coloured_frame() {
        let dir = tempfile::tempdir().unwrap();
        let mut pixels = Vec::new();
        for index in 0..320usize * 240 {
            pixels.extend([
                (index & 255) as u8,
                ((index / 3) & 255) as u8,
                ((index / 7) & 255) as u8,
            ]);
        }
        require_live_ppm(&write(dir.path(), "live.ppm", &pixels)).unwrap();
    }

    #[test]
    fn live_ppm_rejects_wrong_size_and_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let short = dir.path().join("short.ppm");
        fs::write(&short, b"P6\n320 240\n255\nabc").unwrap();
        assert!(require_live_ppm(&short).is_err());
        let small = dir.path().join("small.ppm");
        fs::write(&small, b"P6\n2 2\n255\n\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        assert!(require_live_ppm(&small).is_err());
    }

    #[test]
    fn relocated_track_play_command_is_required() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("cd.csv");
        fs::write(&log, "command,param_len,params\n0x03,1,07\n").unwrap();
        require_track(&log, 7).unwrap();
        assert!(require_track(&log, 8).is_err());
    }

    #[test]
    fn cue_names_and_shell_words() {
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("a.cue");
        fs::write(&cue, "FILE \"My Disc.bin\" BINARY\n  TRACK 01 MODE2/2352\n").unwrap();
        assert_eq!(cue_bin(&cue).unwrap(), dir.path().join("My Disc.bin"));
        assert_eq!(
            shlex_join(&["a b".to_string(), "c".to_string(), "it's".to_string()]),
            "'a b' c 'it'\"'\"'s'"
        );
        fs::write(&cue, "TRACK 01\n").unwrap();
        assert!(cue_bin(&cue).is_err());
    }

    #[test]
    fn every_route_is_built_from_the_table() {
        let names = [
            "CORTEX",
            "VOXIDE",
            "NITROXIDE",
            "CELESTE COLLECTION",
            "PSXCEL",
            "PSOXIDE ARCADE",
        ];
        let entries = toc_rows(&table(&names, true)).unwrap();
        let all = routes(&entries).unwrap();
        assert_eq!(all.len(), 9);
        assert_eq!(all[0].name, "voxide");
        assert!(all.iter().all(|route| route.presses.contains(":cross:12")));
        let magikarp = all.last().unwrap();
        assert_eq!(magikarp.cdda_owner, Some("PSOXIDE ARCADE"));
        assert_eq!(magikarp.launcher_count, 2);
        // A table missing a program cannot produce a route at all.
        let short = toc_rows(&table(&["VOXIDE"], false)).unwrap();
        assert!(routes(&short).is_err());
    }
}
