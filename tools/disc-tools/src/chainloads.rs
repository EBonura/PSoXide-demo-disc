//! Deterministically chain-load and exercise the release-critical disc
//! entries: each target is launched from the carousel twice through the
//! headless emulator, the two replays must agree byte for byte on everything
//! they logged, and the logs must show the loader reading the program, the
//! program running inside its own checksummed PS-X EXE, and (for the games
//! with a gameplay threshold) textured frames sustained long enough to prove
//! the route reached gameplay and not only a menu.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use crate::args::Args;
use crate::disc::{self, FLAG_HIDDEN, USER_DATA_BYTES};
use crate::programs::read_rows;
use crate::replay::{
    number, parse_int, pattern, run_merged, setloc_lba, stdout_core, Csv, Scratch,
};
use crate::util::{fnv1a32, resolve, sha256_file, Error, Result};

const DEFAULT_STEPS: u64 = 700_000_000;
/// Every pressing carries the core entries; HL=1, CS=1 and HK=1 add the others.
const CORE_TARGETS: [&str; 2] = ["CORTEX IGNITION", "QUAKE SHAREWARE"];
const OPTIONAL_TARGETS: [&str; 3] = ["HALF-LIFE", "COUNTER-STRIKE", "HOLLOW KNIGHT"];
const TARGETS: [&str; 5] = [
    "CORTEX IGNITION",
    "QUAKE SHAREWARE",
    "HALF-LIFE",
    "COUNTER-STRIKE",
    "HOLLOW KNIGHT",
];

/// Markers the target must print exactly once after the launcher's own two.
fn target_markers(target: &str) -> &'static [&'static str] {
    match target {
        // cs-psx is built on hl-psx's renderer and keeps its log prefix.
        "HALF-LIFE" | "COUNTER-STRIKE" => &["hl-psx: booting renderer"],
        "QUAKE SHAREWARE" => &[
            "quake-psx: all-Rust PSoXide boot",
            "quake-psx: Rust Start map resident",
        ],
        _ => &[],
    }
}

/// Output that fails the run if it appears at all.
fn target_failures(target: &str) -> &'static [&'static str] {
    match target {
        "HALF-LIFE" | "COUNTER-STRIKE" => &[
            "PANIC:",
            "STACK/DATA COLLISION",
            "hl-psx: WORLD.PAK texture stream failed",
            "hl-psx: WORLD.PAK world stream failed",
        ],
        "QUAKE SHAREWARE" => &[
            "PANIC:",
            "STACK/DATA COLLISION",
            "quake-psx: Rust graphics load failed",
            "quake-psx: Rust initial level load failed",
        ],
        _ => &["PANIC:", "STACK/DATA COLLISION"],
    }
}

/// Every target must be seen reading from the disc after the loader finished.
const REQUIRE_RUNTIME_READ: [&str; 5] = TARGETS;

// These are deliberately below the accepted Cortex 0.4 gameplay load
// (259-294 textured triangles and 115 quads in the integration replay), but
// above every frame of Cortex's own menu/loading sequence (zero textured
// triangles and at most 54 quads).
// Requiring both therefore proves that the launcher entered Cortex, Cortex's
// menu accepted input, and a textured model plus the textured world rendered.
const CORTEX_GAMEPLAY_TRIANGLES: u64 = 240;
const CORTEX_GAMEPLAY_QUADS: u64 = 100;
const CORTEX_GAMEPLAY_MIN_FRAMES: usize = 30;
const CORTEX_GAMEPLAY_MAX_FRAME_GAP: i64 = 16;
const CORTEX_GAMEPLAY_MIN_HASHES: usize = 8;
// Measured 2026-09-26 on the 0.4b Comicon build: the intro skip and panels put
// steady gameplay at route tick ~5000; 1.4 billion steps reach tick ~5800.
const CORTEX_STEPS: u64 = 1_500_000_000;
// Remeasured 2026-09-25 on hl-psx d9d3248 (final-6, PGO) from the 13-card
// pressing: its menu draws no textured triangles and at most 69 quads, and the
// tram ride's tunnel now clears 300/150 in only 7 frames of the replay (the
// older build's gate), but 200/100 in 129 frames with 124 distinct hashes.
const HL_GAMEPLAY_TRIANGLES: u64 = 200;
const HL_GAMEPLAY_QUADS: u64 = 100;
const HL_GAMEPLAY_MIN_FRAMES: usize = 30;
const HL_GAMEPLAY_MIN_HASHES: usize = 8;

/// Per entry: minimum textured triangles, quads and rects in one frame, the
/// frames and distinct frame hashes that must clear them, and what they prove.
/// Measured 2026-09-23 on the frozen frontend. cs-psx's own menu draws no
/// textured triangles and at most 47 quads; de_dust2 draws hundreds of
/// triangles from its first frame.
struct Threshold {
    triangles: u64,
    quads: u64,
    rects: u64,
    min_frames: usize,
    min_hashes: usize,
    what: &'static str,
}

fn threshold_gameplay(target: &str) -> Option<Threshold> {
    match target {
        "HALF-LIFE" => Some(Threshold {
            triangles: HL_GAMEPLAY_TRIANGLES,
            quads: HL_GAMEPLAY_QUADS,
            rects: 0,
            min_frames: HL_GAMEPLAY_MIN_FRAMES,
            min_hashes: HL_GAMEPLAY_MIN_HASHES,
            what: "textured train-ride gameplay",
        }),
        "COUNTER-STRIKE" => Some(Threshold {
            triangles: 150,
            quads: 0,
            rects: 0,
            min_frames: 30,
            min_hashes: 8,
            what: "textured de_dust2 gameplay",
        }),
        _ => None,
    }
}

// A polygon-count gate cannot distinguish a correctly rendered room from a
// deterministic frame that draws only its floor and character.  Cortex's
// release start deliberately faces textured world geometry, so require real
// high-frequency detail in the upper playfield as a visual-semantic oracle.
// The ROI excludes the HUD and player silhouette. The broken exterior-facing
// capture measured 92 permille; the foggier 0.4 authored view measures 135.
const CORTEX_GEOMETRY_EDGE_DELTA: u32 = 24;
const CORTEX_GEOMETRY_MIN_EDGE_PERMILLE: u64 = 110;

static TICK_SUMMARY: OnceLock<regex::Regex> = OnceLock::new();
static ROUTE_SUMMARY: OnceLock<regex::Regex> = OnceLock::new();
static VRAM: OnceLock<regex::Regex> = OnceLock::new();
static DISPLAY: OnceLock<regex::Regex> = OnceLock::new();

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    exe_lba: u64,
    image_lba: u64,
    payload_fnv: u32,
    version: String,
    visible_index: usize,
}

#[derive(Debug, Clone, Copy)]
struct Exe {
    load: u64,
    payload_bytes: u64,
}

/// The visible entries and the carousel's card names (CREDITS last).
fn parse_toc(image: &Path) -> Result<(Vec<Entry>, Vec<String>)> {
    let rows = read_rows(image)?;
    let visible: Vec<_> = rows
        .iter()
        .filter(|row| row.flags & FLAG_HIDDEN == 0)
        .collect();
    let mut menu: Vec<String> = visible.iter().map(|row| row.name.clone()).collect();
    if !menu.is_empty() {
        menu.push("CREDITS".into());
    }
    let entries = visible
        .iter()
        .enumerate()
        .map(|(visible_index, row)| Entry {
            name: row.name.clone(),
            exe_lba: u64::from(row.exe_lba),
            image_lba: u64::from(row.image_lba),
            payload_fnv: row.payload_fnv,
            version: row.version.clone(),
            visible_index,
        })
        .collect();
    Ok((entries, menu))
}

/// The entry's PS-X EXE, checked against the table's payload checksum.
fn parse_exe(image: &Path, entry: &Entry) -> Result<Exe> {
    let header = disc::read_user_sectors(image, entry.exe_lba, 1)?;
    ensure!(
        &header[..8] == disc::PSX_EXE_MAGIC,
        "{}: no PS-X EXE at LBA {}",
        entry.name,
        entry.exe_lba
    );
    let pc = u64::from(disc::le32(&header, 0x10));
    let load = u64::from(disc::le32(&header, 0x18));
    let payload_bytes = u64::from(disc::le32(&header, 0x1C));
    ensure!(
        payload_bytes != 0 && payload_bytes % USER_DATA_BYTES as u64 == 0,
        "{}: invalid payload size {payload_bytes}",
        entry.name
    );
    ensure!(
        load <= pc && pc < load + payload_bytes,
        "{}: entry PC is outside its payload",
        entry.name
    );
    let payload = disc::read_user_sectors(
        image,
        entry.exe_lba + 1,
        payload_bytes / USER_DATA_BYTES as u64,
    )?;
    let digest = fnv1a32(&payload);
    ensure!(
        digest == entry.payload_fnv,
        "{}: payload FNV {digest:#010x} != TOC {:#010x}",
        entry.name,
        entry.payload_fnv
    );
    Ok(Exe {
        load,
        payload_bytes,
    })
}

/// The presses that bring card `index` of `count` to the middle and the tick
/// of the launching CROSS.
fn launcher_route(index: usize, count: usize) -> (Vec<String>, i64) {
    let (index, count) = (index as i64, count as i64);
    let left = index;
    let right = (-index).rem_euclid(count);
    let (button, presses) = if left <= right {
        ("left", left)
    } else {
        ("right", right)
    };
    let mut events: Vec<String> = (0..presses)
        .map(|step| format!("{}:{button}:8", 400 + step * 200))
        .collect();
    let cross_tick = 1_000.max(400 + presses * 200 + 200);
    events.push(format!("{cross_tick}:cross:12"));
    (events, cross_tick)
}

fn route_for(target: &str, index: usize, count: usize) -> String {
    let (mut events, cross_tick) = launcher_route(index, count);
    if matches!(
        target,
        "CORTEX IGNITION" | "HALF-LIFE" | "COUNTER-STRIKE" | "HOLLOW KNIGHT"
    ) {
        // These programs have their own menu after the demo-disc carousel.
        // Sparse presses remain deterministic across scene and data loads and
        // enter gameplay without depending on a single timing edge.
        events.extend(
            [400, 800, 1200, 1600, 2000, 2400]
                .map(|offset| format!("{}:cross:12", cross_tick + offset)),
        );
    }
    if target == "CORTEX IGNITION" {
        // Cortex 0.4b (Comicon) opens New Game with an intro that only a held
        // CROSS skips, then three welcome panels that CROSS dismisses.
        events.push(format!("{}:cross:150", cross_tick + 2600));
        events.extend([3000, 3400, 3800].map(|offset| format!("{}:cross:12", cross_tick + offset)));
    }
    events.join(",")
}

/// The intro and panels push Cortex's gameplay past the default budget.
fn steps_for(target: &str, steps: u64) -> u64 {
    if target == "CORTEX IGNITION" {
        steps.max(CORTEX_STEPS)
    } else {
        steps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Gameplay {
    frames: usize,
    sustained: usize,
    hashes: usize,
}

fn unsigned(csv: &Csv, row: &[String], column: &str) -> Result<u64> {
    parse_int(csv.required(row, column)?, 10)
}

fn cortex_gameplay_evidence(path: &Path) -> Result<Gameplay> {
    let csv = Csv::read(path)?;
    let mut qualifying: Vec<(i64, String)> = Vec::new();
    for row in &csv.rows {
        if unsigned(&csv, row, "textured_tris")? >= CORTEX_GAMEPLAY_TRIANGLES
            && unsigned(&csv, row, "textured_quads")? >= CORTEX_GAMEPLAY_QUADS
        {
            qualifying.push((
                unsigned(&csv, row, "route_tick")? as i64,
                csv.required(row, "frame_draw_hash")?.to_string(),
            ));
        }
    }
    let mut longest = 0;
    let mut run = 0;
    let mut previous: Option<i64> = None;
    for (tick, _) in &qualifying {
        run = match previous {
            Some(before) if tick - before <= CORTEX_GAMEPLAY_MAX_FRAME_GAP => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        previous = Some(*tick);
    }
    let hashes = qualifying
        .iter()
        .map(|(_, digest)| digest)
        .collect::<std::collections::HashSet<_>>()
        .len();
    ensure!(
        qualifying.len() >= CORTEX_GAMEPLAY_MIN_FRAMES,
        "CORTEX IGNITION: only {} textured gameplay frames; need {CORTEX_GAMEPLAY_MIN_FRAMES}",
        qualifying.len()
    );
    ensure!(
        longest >= CORTEX_GAMEPLAY_MIN_FRAMES,
        "CORTEX IGNITION: textured frames were not sustained ({longest} in one run)"
    );
    ensure!(
        hashes >= CORTEX_GAMEPLAY_MIN_HASHES,
        "CORTEX IGNITION: textured gameplay did not animate ({hashes} distinct frame hashes)"
    );
    Ok(Gameplay {
        frames: qualifying.len(),
        sustained: longest,
        hashes,
    })
}

fn threshold_gameplay_evidence(path: &Path, target: &str) -> Result<Gameplay> {
    let Some(limits) = threshold_gameplay(target) else {
        bail!("{target}: no gameplay threshold");
    };
    let csv = Csv::read(path)?;
    let mut hashes = std::collections::HashSet::new();
    let mut frames = 0;
    for row in &csv.rows {
        if unsigned(&csv, row, "textured_tris")? >= limits.triangles
            && unsigned(&csv, row, "textured_quads")? >= limits.quads
            && (limits.rects == 0 || unsigned(&csv, row, "textured_rects")? >= limits.rects)
        {
            frames += 1;
            hashes.insert(csv.required(row, "frame_draw_hash")?.to_string());
        }
    }
    ensure!(
        frames >= limits.min_frames && hashes.len() >= limits.min_hashes,
        "{target}: route did not sustain {} ({frames} frames, {} hashes)",
        limits.what,
        hashes.len()
    );
    Ok(Gameplay {
        frames,
        sustained: frames,
        hashes: hashes.len(),
    })
}

#[cfg(test)]
fn hl_gameplay_evidence(path: &Path) -> Result<Gameplay> {
    threshold_gameplay_evidence(path, "HALF-LIFE")
}

/// Edge permille of the upper playfield in the final display dump.
fn cortex_geometry_evidence(path: &Path) -> Result<u64> {
    let payload = fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    static HEADER: OnceLock<regex::bytes::Regex> = OnceLock::new();
    let header = HEADER
        .get_or_init(|| {
            regex::bytes::Regex::new(r"(?-u)^P6\s+(\d+)\s+(\d+)\s+255\s").expect("static regex")
        })
        .captures(&payload);
    let Some(header) = header else {
        bail!("CORTEX IGNITION: malformed P6 display dump");
    };
    let dimension = |group: usize| -> Result<usize> {
        String::from_utf8_lossy(&header[group])
            .parse()
            .map_err(|_| Error("CORTEX IGNITION: malformed P6 display dump".into()))
    };
    let (width, height) = (dimension(1)?, dimension(2)?);
    let pixels = &payload[header.get(0).expect("whole match").end()..];
    ensure!(
        pixels.len() == width * height * 3,
        "CORTEX IGNITION: truncated display dump"
    );
    ensure!(
        (width, height) == (320, 240),
        "CORTEX IGNITION: unexpected display size {width}x{height}"
    );
    let (mut edge_pixels, mut compared) = (0u64, 0u64);
    for y in 36..150 {
        for x in 32..287 {
            let offset = (y * width + x) * 3;
            let delta: u32 = (0..3)
                .map(|channel| {
                    u32::from(pixels[offset + channel].abs_diff(pixels[offset + 3 + channel]))
                })
                .sum();
            edge_pixels += u64::from(delta >= CORTEX_GEOMETRY_EDGE_DELTA);
            compared += 1;
        }
    }
    let edge_permille = edge_pixels * 1000 / compared;
    ensure!(
        edge_permille >= CORTEX_GEOMETRY_MIN_EDGE_PERMILLE,
        "CORTEX IGNITION: upper playfield lacks textured geometry ({edge_permille} edge \
         permille; need {CORTEX_GEOMETRY_MIN_EDGE_PERMILLE})"
    );
    Ok(edge_permille)
}

/// What one replay observed, plus the logs it wrote.
#[derive(Debug, Clone)]
struct Replay {
    stdout: String,
    tick: u64,
    cycles: u64,
    pc_final: u64,
    route_ticks: u64,
    pad_polls: u64,
    vram: String,
    display: String,
    width: u64,
    height: u64,
    samples: u64,
    gameplay: Gameplay,
    geometry_edge_permille: u64,
    route: PathBuf,
    cd: PathBuf,
    gpu: PathBuf,
    pc: PathBuf,
    display_ppm: PathBuf,
}

#[allow(clippy::too_many_arguments)]
fn run_once(
    frontend: &Path,
    cue: &Path,
    target: &Entry,
    exe: Exe,
    menu_count: usize,
    steps: u64,
    root: &Path,
    label: &str,
) -> Result<Replay> {
    let run_dir = root.join(format!(
        "{}-{label}",
        target.name.to_lowercase().replace(' ', "-")
    ));
    fs::create_dir(&run_dir)?;
    let route_log = run_dir.join("route.csv");
    let cd = run_dir.join("cd.csv");
    let gpu = run_dir.join("gpu.csv");
    let pc = run_dir.join("pc.csv");
    let display_ppm = run_dir.join("display.ppm");
    let mut command = Command::new(frontend);
    command
        .args(["launch", "--embedded-playtest", "--path"])
        .arg(cue)
        .arg("--steps")
        .arg(steps_for(&target.name, steps).to_string())
        .arg("--press")
        .arg(route_for(&target.name, target.visible_index, menu_count))
        .arg("--route-log")
        .arg(&route_log)
        .arg("--cd-command-log")
        .arg(&cd)
        .arg("--gpu-frame-stats-log")
        .arg(&gpu)
        .arg("--pc-sample-log")
        .arg(&pc)
        .arg("--dump-display")
        .arg(&display_ppm)
        .arg("--dump-hash");
    let (code, stdout) = run_merged(&mut command, &root.join(format!("{label}.out")), None)?;
    let name = &target.name;
    ensure!(code == 0, "{name} {label} failed:\n{stdout}");
    let launcher = ["launcher: booted", "launcher: chain-loading"];
    for marker in launcher
        .into_iter()
        .chain(target_markers(name).iter().copied())
    {
        ensure!(
            stdout.matches(marker).count() == 1,
            "{name}: expected one {marker:?}"
        );
    }
    for marker in target_failures(name) {
        ensure!(
            !stdout.contains(marker),
            "{name}: found failure marker {marker:?}"
        );
    }
    let tick = pattern(
        &TICK_SUMMARY,
        r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)",
    )
    .captures(&stdout);
    let route = pattern(&ROUTE_SUMMARY, r"route-ticks=(\d+)\s+port1-polls=(\d+)").captures(&stdout);
    let vram = pattern(&VRAM, r"vram_fnv1a_64=(0x[0-9a-f]+)").captures(&stdout);
    let display = pattern(
        &DISPLAY,
        r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)",
    )
    .captures(&stdout);
    let (Some(tick), Some(route), Some(vram), Some(display)) = (tick, route, vram, display) else {
        bail!("{name}: incomplete emulator summary");
    };
    let pc_final = parse_int(&tick[3], 16)?;
    ensure!(
        exe.load <= pc_final && pc_final < exe.load + exe.payload_bytes,
        "{name}: final PC {pc_final:#010x} outside payload"
    );
    let csv = Csv::read(&pc)?;
    let mut samples = 0;
    for row in &csv.rows {
        let sampled = parse_int(csv.required(row, "pc")?, 16)?;
        if exe.load <= sampled && sampled < exe.load + exe.payload_bytes {
            samples += unsigned(&csv, row, "samples")?;
        }
    }
    ensure!(
        samples != 0,
        "{name}: PC sampler never observed payload code"
    );
    let gameplay = if name == "CORTEX IGNITION" {
        cortex_gameplay_evidence(&gpu)?
    } else if threshold_gameplay(name).is_some() {
        threshold_gameplay_evidence(&gpu, name)?
    } else {
        Gameplay::default()
    };
    let geometry_edge_permille = if name == "CORTEX IGNITION" {
        cortex_geometry_evidence(&display_ppm)?
    } else {
        0
    };
    Ok(Replay {
        stdout: stdout_core(&stdout),
        tick: number(&tick[1], 10)?,
        cycles: number(&tick[2], 10)?,
        pc_final,
        route_ticks: number(&route[1], 10)?,
        pad_polls: number(&route[2], 10)?,
        vram: vram[1].to_string(),
        display: display[1].to_string(),
        width: number(&display[2], 10)?,
        height: number(&display[3], 10)?,
        samples,
        gameplay,
        geometry_edge_permille,
        route: route_log,
        cd,
        gpu,
        pc,
        display_ppm,
    })
}

/// The CD log shows the loader reading the program header and payload, and
/// the program reading from the disc after it finished loading. Returns the
/// command count and that first runtime read.
fn cd_evidence(path: &Path, entry: &Entry, exe: Exe) -> Result<(usize, Option<i64>)> {
    let csv = Csv::read(path)?;
    let mut read_starts: Vec<i64> = Vec::new();
    for (index, row) in csv.rows.iter().enumerate() {
        let Some(lba) = setloc_lba(&csv, row)? else {
            continue;
        };
        let mut commands = std::collections::HashSet::new();
        for following in &csv.rows[index + 1..] {
            let command = csv.required(following, "command")?;
            if command == "0x02" {
                break;
            }
            commands.insert(command);
        }
        if commands.contains("0x06") {
            read_starts.push(lba);
        }
    }
    let exe_lba = entry.exe_lba as i64;
    ensure!(
        read_starts.contains(&exe_lba) && read_starts.contains(&(exe_lba + 1)),
        "{}: loader did not read header and payload",
        entry.name
    );
    let payload_end = exe_lba + 1 + (exe.payload_bytes / USER_DATA_BYTES as u64) as i64;
    let runtime = read_starts.iter().copied().find(|lba| *lba >= payload_end);
    ensure!(
        runtime.is_some() || !REQUIRE_RUNTIME_READ.contains(&entry.name.as_str()),
        "{}: no post-loader runtime CD read",
        entry.name
    );
    Ok((csv.rows.len(), runtime))
}

fn compare(first: &Replay, second: &Replay, target: &str) -> Result<()> {
    let fields = |replay: &Replay| -> Vec<(&'static str, String)> {
        vec![
            ("stdout", format!("{:?}", replay.stdout)),
            ("tick", replay.tick.to_string()),
            ("cycles", replay.cycles.to_string()),
            ("pc_final", replay.pc_final.to_string()),
            ("route_ticks", replay.route_ticks.to_string()),
            ("pad_polls", replay.pad_polls.to_string()),
            ("vram", format!("{:?}", replay.vram)),
            ("display", format!("{:?}", replay.display)),
            ("width", replay.width.to_string()),
            ("height", replay.height.to_string()),
            ("samples", replay.samples.to_string()),
            ("gameplay_frames", replay.gameplay.frames.to_string()),
            ("gameplay_sustained", replay.gameplay.sustained.to_string()),
            ("gameplay_hashes", replay.gameplay.hashes.to_string()),
            (
                "geometry_edge_permille",
                replay.geometry_edge_permille.to_string(),
            ),
        ]
    };
    for ((key, a), (_, b)) in fields(first).into_iter().zip(fields(second)) {
        ensure!(
            a == b,
            "{target}: {key} differs between replays: {a} != {b}"
        );
    }
    let logs = [
        ("route", &first.route, &second.route),
        ("cd", &first.cd, &second.cd),
        ("gpu", &first.gpu, &second.gpu),
        ("pc", &first.pc, &second.pc),
        ("display_ppm", &first.display_ppm, &second.display_ppm),
    ];
    for (key, a, b) in logs {
        ensure!(
            sha256_file(a)? == sha256_file(b)?,
            "{target}: {key} log differs between replays"
        );
    }
    Ok(())
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(
        raw,
        &["frontend", "cue", "steps", "target", "artifact-dir"],
        &[],
    )?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    for target in args.all("target") {
        ensure!(
            TARGETS.contains(&target),
            "argument --target: invalid choice: {target:?} (choose from {})",
            TARGETS.join(", ")
        );
    }
    let steps = match args.int("steps")? {
        None => DEFAULT_STEPS,
        Some(steps) => u64::try_from(steps)
            .map_err(|_| Error(format!("--steps must not be negative, got {steps}")))?,
    };
    let frontend = resolve(&args.require_path("frontend")?)?;
    let cue = resolve(&args.require_path("cue")?)?;
    let image = disc::image_for_cue(&cue)?;
    let (entries, menu) = parse_toc(&image)?;
    // With no --target: the core entries plus every optional one pressed.
    let mut targets: Vec<String> = args.all("target").iter().map(|t| t.to_string()).collect();
    if targets.is_empty() {
        targets.extend(CORE_TARGETS.map(String::from));
        targets.extend(
            OPTIONAL_TARGETS
                .iter()
                .filter(|name| entries.iter().any(|entry| entry.name == **name))
                .map(|name| name.to_string()),
        );
    }
    println!("frontend SHA-256: {}", sha256_file(&frontend)?);
    println!("cue SHA-256: {}", sha256_file(&cue)?);
    println!("bin SHA-256: {}", sha256_file(&image)?);
    println!("menu ({}): {}", menu.len(), menu.join(", "));

    let (root, _scratch) = match args.path("artifact-dir") {
        None => {
            let scratch = Scratch::new("psoxide-release-chainload")?;
            (scratch.0.clone(), Some(scratch))
        }
        Some(dir) => {
            let root = if dir.is_absolute() {
                dir
            } else {
                std::env::current_dir()?.join(dir)
            };
            ensure!(
                !root.exists() || fs::read_dir(&root)?.next().is_none(),
                "artifact directory is not empty: {}",
                root.display()
            );
            fs::create_dir_all(&root)?;
            (root, None)
        }
    };
    for name in &targets {
        let Some(entry) = entries.iter().find(|entry| &entry.name == name) else {
            bail!("required visible entry is absent: {name}");
        };
        let exe = parse_exe(&image, entry)?;
        let first = run_once(
            &frontend,
            &cue,
            entry,
            exe,
            menu.len(),
            steps,
            &root,
            "first",
        )?;
        let second = run_once(
            &frontend,
            &cue,
            entry,
            exe,
            menu.len(),
            steps,
            &root,
            "second",
        )?;
        compare(&first, &second, name)?;
        let (commands, runtime_lba) = cd_evidence(&first.cd, entry, exe)?;
        println!(
            "{name}: PASS route={} version={} EXE-LBA={} image-LBA={} payload-FNV={:#010x} \
             PC={:#010x} samples={} CD={commands} runtime-read={} VRAM={} display={} \
             gameplay={}/{} hashes={} geometry-edge={}permille",
            route_for(name, entry.visible_index, menu.len()),
            if entry.version.is_empty() {
                "-"
            } else {
                &entry.version
            },
            entry.exe_lba,
            entry.image_lba,
            entry.payload_fnv,
            first.pc_final,
            first.samples,
            runtime_lba.map_or("None".to_string(), |lba| lba.to_string()),
            first.vram,
            first.display,
            first.gameplay.frames,
            first.gameplay.sustained,
            first.gameplay.hashes,
            first.geometry_edge_permille,
        );
    }
    println!("all release-critical chain-load replays are byte-deterministic");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_gpu(dir: &Path, rows: &[(i64, u64, u64, String)]) -> PathBuf {
        let path = dir.join("gpu.csv");
        let mut text = String::from("route_tick,textured_tris,textured_quads,frame_draw_hash\r\n");
        for (tick, tris, quads, hash) in rows {
            text.push_str(&format!("{tick},{tris},{quads},{hash}\r\n"));
        }
        fs::write(&path, text).unwrap();
        path
    }

    fn write_display(dir: &Path, textured: bool) -> PathBuf {
        let path = dir.join("display.ppm");
        let mut data = b"P6\n320 240\n255\n".to_vec();
        for y in 0..240 {
            for x in 0..320 {
                let in_oracle = (32..288).contains(&x) && (36..150).contains(&y);
                let value = if textured && in_oracle && x % 2 == 0 {
                    32
                } else {
                    96
                };
                data.extend([value, value, value]);
            }
        }
        fs::write(&path, data).unwrap();
        path
    }

    fn message<T>(result: Result<T>) -> String {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(error) => error.0,
        }
    }

    #[test]
    fn cortex_route_enters_internal_menu_after_launcher() {
        assert_eq!(
            route_for("CORTEX IGNITION", 0, 11),
            "1000:cross:12,1400:cross:12,1800:cross:12,2200:cross:12,\
             2600:cross:12,3000:cross:12,3400:cross:12,3600:cross:150,\
             4000:cross:12,4400:cross:12,4800:cross:12"
        );
        assert_eq!(
            route_for("HALF-LIFE", 1, 11),
            "400:left:8,1000:cross:12,1400:cross:12,\
             1800:cross:12,2200:cross:12,2600:cross:12,3000:cross:12,\
             3400:cross:12"
        );
    }

    #[test]
    fn launcher_route_takes_the_short_way_round() {
        // Index 9 of 11 is two cards behind the start: two RIGHT presses.
        assert_eq!(
            launcher_route(9, 11),
            (
                vec![
                    "400:right:8".to_string(),
                    "600:right:8".to_string(),
                    "1000:cross:12".to_string()
                ],
                1000
            )
        );
        assert_eq!(steps_for("CORTEX IGNITION", 1), CORTEX_STEPS);
        assert_eq!(steps_for("QUAKE SHAREWARE", 1), 1);
    }

    #[test]
    fn sustained_textured_gameplay_passes() {
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<_> = (0..35)
            .map(|index| (100 + index * 8, 700, 100, format!("0x{:016x}", index + 1)))
            .collect();
        let evidence = cortex_gameplay_evidence(&write_gpu(dir.path(), &rows)).unwrap();
        assert_eq!(
            evidence,
            Gameplay {
                frames: 35,
                sustained: 35,
                hashes: 35
            }
        );
    }

    #[test]
    fn half_life_train_ride_gameplay_passes() {
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<_> = (0..35)
            .map(|index| (100 + index * 4, 423, 246, format!("0x{:016x}", index + 1)))
            .collect();
        let evidence = hl_gameplay_evidence(&write_gpu(dir.path(), &rows)).unwrap();
        assert_eq!(
            evidence,
            Gameplay {
                frames: 35,
                sustained: 35,
                hashes: 35
            }
        );
    }

    #[test]
    fn menu_like_frames_fail() {
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<_> = (0..100)
            .map(|index| (index, 0, 31, "0x1".to_string()))
            .collect();
        let error = message(cortex_gameplay_evidence(&write_gpu(dir.path(), &rows)));
        assert!(error.contains("only 0 textured"), "{error}");
    }

    #[test]
    fn isolated_frames_are_not_sustained() {
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<_> = (0..35)
            .map(|index| (index * 32, 700, 100, format!("0x{:x}", index + 1)))
            .collect();
        let error = message(cortex_gameplay_evidence(&write_gpu(dir.path(), &rows)));
        assert!(error.contains("not sustained"), "{error}");
    }

    #[test]
    fn static_frame_hash_fails() {
        let dir = tempfile::tempdir().unwrap();
        let rows: Vec<_> = (0..35)
            .map(|index| (index * 8, 700, 100, "0x1".to_string()))
            .collect();
        let error = message(cortex_gameplay_evidence(&write_gpu(dir.path(), &rows)));
        assert!(error.contains("did not animate"), "{error}");
    }

    #[test]
    fn textured_upper_playfield_passes_geometry_oracle() {
        let dir = tempfile::tempdir().unwrap();
        let permille = cortex_geometry_evidence(&write_display(dir.path(), true)).unwrap();
        assert!(permille >= CORTEX_GEOMETRY_MIN_EDGE_PERMILLE);
    }

    #[test]
    fn flat_upper_playfield_fails_geometry_oracle() {
        let dir = tempfile::tempdir().unwrap();
        let error = message(cortex_geometry_evidence(&write_display(dir.path(), false)));
        assert!(error.contains("lacks textured geometry"), "{error}");
    }

    #[test]
    fn replays_must_agree_on_every_field_and_log() {
        let dir = tempfile::tempdir().unwrap();
        let mut logs = Vec::new();
        for name in ["route", "cd", "gpu", "pc", "display"] {
            let path = dir.path().join(name);
            fs::write(&path, name).unwrap();
            logs.push(path);
        }
        let replay = Replay {
            stdout: "same".into(),
            tick: 1,
            cycles: 2,
            pc_final: 3,
            route_ticks: 4,
            pad_polls: 5,
            vram: "0x1".into(),
            display: "0x2".into(),
            width: 320,
            height: 240,
            samples: 6,
            gameplay: Gameplay::default(),
            geometry_edge_permille: 0,
            route: logs[0].clone(),
            cd: logs[1].clone(),
            gpu: logs[2].clone(),
            pc: logs[3].clone(),
            display_ppm: logs[4].clone(),
        };
        compare(&replay, &replay.clone(), "T").unwrap();
        let mut other = replay.clone();
        other.cycles = 9;
        assert!(message(compare(&replay, &other, "T")).contains("cycles differs"));
        let mut other = replay.clone();
        fs::write(dir.path().join("other"), "changed").unwrap();
        other.gpu = dir.path().join("other");
        assert!(message(compare(&replay, &other, "T")).contains("gpu log differs"));
    }

    #[test]
    fn cd_log_must_show_loader_and_runtime_reads() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("cd.csv");
        // Seeks are BCD minute/second/frame: LBA = (m*60 + s)*75 + f - 150,
        // so LBA 50 is 00:02:50, LBA 51 is 00:02:51 and LBA 80 is 00:03:05.
        let text = "command,param_len,params\n\
                    0x02,3,00 02 50\n0x06,0,\n\
                    0x02,3,00 02 51\n0x06,0,\n\
                    0x02,3,00 03 05\n0x06,0,\n";
        fs::write(&log, text).unwrap();
        let entry = Entry {
            name: "QUAKE SHAREWARE".into(),
            exe_lba: 50,
            image_lba: 60,
            payload_fnv: 0,
            version: String::new(),
            visible_index: 0,
        };
        let exe = Exe {
            load: 0x8001_0000,
            payload_bytes: 2048,
        };
        // 00:03:05 is LBA 80; payload ends at 50 + 1 + 1 = 52.
        assert_eq!(cd_evidence(&log, &entry, exe).unwrap(), (6, Some(80)));
        fs::write(&log, "command,param_len,params\n0x02,3,00 02 50\n0x06,0,\n").unwrap();
        assert!(message(cd_evidence(&log, &entry, exe)).contains("did not read header"));
        fs::write(
            &log,
            "command,param_len,params\n0x02,3,00 02 50\n0x06,0,\n0x02,3,00 02 51\n0x06,0,\n",
        )
        .unwrap();
        assert!(message(cd_evidence(&log, &entry, exe)).contains("no post-loader runtime"));
    }
}
