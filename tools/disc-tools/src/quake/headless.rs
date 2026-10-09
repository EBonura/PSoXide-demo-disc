//! The legacy headless chain-load check: boot a combined disc twice through
//! the emulator, select Quake in the carousel by its table row, and require
//! both replays to agree on everything they observed plus runtime evidence
//! that Quake really loaded. No Makefile target runs this any more (the
//! `chainloads` subcommand replaced it) but it stays for the single-title
//! Quake gate with its own receipt.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use crate::args::Args;
use crate::disc::{self, le32, text_field, TOC_ENTRY_BYTES, TOC_HEADER_BYTES, TOC_NAME_BYTES};
use crate::replay::{number, parse_int, pattern, setloc_lba, stdout_core, Csv, Scratch};
use crate::util::{resolve, sha256_file, Error, Result};

const STEPS: &str = "500000000";
const PRESS_ROUTE: &str = "400:right:8,600:right:8,1000:cross:12";
const EXPECTED_TICK: u64 = 500_000_000;
const EXPECTED_DISPLAY: (u64, u64) = (320, 240);
/// The standard pressing hides the unfinished Cortex card, leaving eight
/// visible programs plus CREDITS. The private HL pressing exposes Cortex and
/// adds Half-Life, so its caller raises this to eleven. Quake stays
/// immediately before CREDITS in both layouts.
const DEFAULT_MENU_ENTRIES: usize = 9;
/// The frame both replays end on, hashed by the emulator. Each pressing needs
/// its own pair because its carousel table gives the launcher a different
/// amount of work before the fixed instruction budget reaches Quake.
/// Recompute these when the Quake, launcher, or ordinary-program PSoXide pin
/// moves.
///
/// Absolute cycle counts, route ticks, pad polls, CD command totals and log
/// digests are deliberately NOT pinned here. They shift with the launcher
/// binary, which changes on every commit to this repo (DISC_VERSION is `git
/// describe`), so pinning them would have made the default gate fail on
/// unrelated work. They are held to run-to-run equality instead, which is
/// what determinism means.
const EXPECTED_FRAME_FNV_BY_MENU_ENTRIES: [(usize, (&str, &str)); 3] = [
    // Public pressing before 2026-09-03: the Cortex card was hidden.
    (
        DEFAULT_MENU_ENTRIES,
        ("0x1db7984ba55cc00a", "0x202e95d7fce1debf"),
    ),
    // Public pressing since 2026-09-03: Cortex is on the carousel.
    (10, ("0x06909494541b63a7", "0x7f09fe1a7973013a")),
    // Private Half-Life pressing, measured with the clean Comicon launcher.
    (11, ("0x1f3fe9f43ef41800", "0xf782598f822c9017")),
];
const DETERMINISTIC_FIELDS: [&str; 9] = [
    "tick",
    "cycles",
    "pc_final",
    "route_ticks",
    "pad_polls",
    "vram_fnv",
    "display_fnv",
    "display_width",
    "display_height",
];
const LOG_KINDS: [&str; 6] = ["route", "cd", "gpu", "pc", "pc_callsite", "pc_window"];
const MARKERS: [&str; 4] = [
    "launcher: booted",
    "launcher: chain-loading",
    "quake-psx: all-Rust PSoXide boot",
    "quake-psx: Rust Start map resident",
];
const FAILURE_MARKERS: [&str; 4] = [
    "quake-psx: Rust graphics load failed",
    "quake-psx: Rust initial level load failed",
    "STACK/DATA COLLISION",
    "PANIC:",
];

const TOC_PAYLOAD_FNV_AT: usize = 36;
const TOC_FLAGS_AT: usize = disc::TOC_FLAGS_AT;
const TOC_VERSION_AT: usize = disc::TOC_VERSION_AT;
const QUAKE_ENTRY: &str = "QUAKE SHAREWARE";

fn frame_pins(menu_entries: usize) -> Option<(&'static str, &'static str)> {
    EXPECTED_FRAME_FNV_BY_MENU_ENTRIES
        .iter()
        .find(|(n, _)| *n == menu_entries)
        .map(|(_, pins)| *pins)
}

/// How many times the headless route presses `button`.
fn route_button_count(button: &str) -> usize {
    PRESS_ROUTE
        .split(',')
        .filter(|press| press.split(':').nth(1) == Some(button))
        .count()
}

/// The table's own record of the Quake payload, held against the receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Payload {
    exe_lba: u64,
    payload_fnv1a32: String,
    menu_version: String,
}

/// Where QUAKE SHAREWARE sits in the carousel, and what the table says it is.
///
/// Returns the selected index, the visible entry names, and the table's own
/// record of the payload, so the caller can hold both against the receipt.
fn quake_menu_entry(
    image: &Path,
    quake_lba: u64,
    expected_menu_entries: usize,
) -> Result<(usize, Vec<String>, Payload)> {
    let toc = disc::read_user_sectors(image, disc::TOC_LBA, disc::TOC_SECTORS)?;
    ensure!(
        &toc[..8] == disc::TOC_MAGIC,
        "{}: missing {} at LBA {}",
        image.display(),
        String::from_utf8_lossy(disc::TOC_MAGIC),
        disc::TOC_LBA
    );
    let count = le32(&toc, 8) as usize;
    ensure!(
        count != 0 && count <= disc::TOC_MAX_ENTRIES,
        "{}: invalid demo table entry count {count}",
        image.display()
    );

    let mut visible: Vec<(String, u64)> = Vec::new();
    let mut payload: Option<Payload> = None;
    for index in 0..count {
        let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
        let entry = &toc[at..at + TOC_ENTRY_BYTES];
        let name = text_field(&entry[..TOC_NAME_BYTES])?;
        let exe_lba = u64::from(le32(entry, TOC_NAME_BYTES));
        let flags = le32(entry, TOC_FLAGS_AT);
        if name == QUAKE_ENTRY {
            ensure!(
                payload.is_none(),
                "{}: more than one {QUAKE_ENTRY} table entry",
                image.display()
            );
            ensure!(
                flags & disc::FLAG_HIDDEN == 0,
                "{}: {QUAKE_ENTRY} is hidden from the carousel",
                image.display()
            );
            payload = Some(Payload {
                exe_lba,
                payload_fnv1a32: format!("0x{:08x}", le32(entry, TOC_PAYLOAD_FNV_AT)),
                menu_version: text_field(
                    &entry[TOC_VERSION_AT..TOC_VERSION_AT + disc::TOC_VERSION_BYTES],
                )?,
            });
        }
        if flags & disc::FLAG_HIDDEN == 0 {
            visible.push((name, exe_lba));
        }
    }
    let Some(payload) = payload else {
        bail!(
            "{}: no {QUAKE_ENTRY} entry in the disc table",
            image.display()
        );
    };
    // The launcher appends its own CREDITS card whenever there is room for it.
    if !visible.is_empty() && visible.len() < disc::TOC_MAX_ENTRIES {
        visible.push(("CREDITS".to_string(), 0));
    }
    ensure!(
        visible.len() == expected_menu_entries,
        "{}: carousel has {} visible entries, expected {expected_menu_entries}: {}",
        image.display(),
        visible.len(),
        visible
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let right_presses = route_button_count("right");
    let cross_presses = route_button_count("cross");
    ensure!(
        right_presses == 2 && cross_presses == 1,
        "headless route no longer has two RIGHT presses and one CROSS"
    );
    // Each RIGHT press moves one card further round; the carousel wraps.
    let selected = (visible.len() - right_presses % visible.len()) % visible.len();
    let (selected_name, selected_lba) = &visible[selected];
    ensure!(
        selected_name == QUAKE_ENTRY && *selected_lba == quake_lba,
        "menu route selects {selected_name:?} at LBA {selected_lba}, not Quake at LBA {quake_lba}"
    );
    let expected_position = expected_menu_entries - 1;
    ensure!(
        selected + 1 == expected_position,
        "{QUAKE_ENTRY} is carousel entry {}, expected {expected_position}",
        selected + 1
    );
    Ok((
        selected,
        visible.into_iter().map(|(name, _)| name).collect(),
        payload,
    ))
}

/// A field of the receipt, or an error naming it (Python's `KeyError`).
fn field<'a>(container: &'a Value, key: &str) -> Result<&'a Value> {
    container
        .get(key)
        .ok_or_else(|| Error(format!("receipt has no field {key:?}")))
}

fn unsigned(value: &Value, key: &str) -> Result<u64> {
    value.as_u64().ok_or_else(|| {
        Error(format!(
            "receipt field {key:?} must be a non-negative integer"
        ))
    })
}

/// The disc the emulator just ran has to be the one the receipt describes.
fn require_payload_identity(payload: &Payload, output: &Value) -> Result<()> {
    let recorded = field(output, "quake_toc")?;
    let checks = [
        ("exe_lba", Value::from(payload.exe_lba)),
        (
            "payload_fnv1a32",
            Value::String(payload.payload_fnv1a32.clone()),
        ),
        ("menu_version", Value::String(payload.menu_version.clone())),
    ];
    for (name, found) in checks {
        let expected = field(recorded, name)?;
        ensure!(
            &found == expected,
            "disc table {name} {found} does not match the receipt's {expected}"
        );
    }
    ensure!(
        output.get("embedded_quake_matches_input_except_msf") == Some(&Value::Bool(true)),
        "receipt does not claim the embedded Quake image is the pinned one"
    );
    ensure!(
        unsigned(
            field(output, "embedded_quake_data_sectors")?,
            "embedded_quake_data_sectors"
        )? > 0,
        "receipt records no embedded Quake data sectors"
    );
    Ok(())
}

/// Entry PC, load address and payload size of the Quake EXE in the table.
fn embedded_exe_evidence(image: &Path, quake_lba: u64) -> Result<(u64, u64, u64)> {
    let header = disc::read_user_sectors(image, quake_lba, 1)?;
    ensure!(
        &header[..8] == disc::PSX_EXE_MAGIC,
        "no PS-X EXE at Quake table LBA {quake_lba}"
    );
    let pc = u64::from(le32(&header, 0x10));
    let load_addr = u64::from(le32(&header, 0x18));
    let payload_bytes = u64::from(le32(&header, 0x1C));
    ensure!(
        payload_bytes != 0 && payload_bytes % disc::USER_DATA_BYTES as u64 == 0,
        "invalid Quake payload size {payload_bytes}"
    );
    ensure!(
        load_addr <= pc && pc < load_addr + payload_bytes,
        "Quake entry PC {pc:#010x} is outside payload {load_addr:#010x}..{:#010x}",
        load_addr + payload_bytes
    );
    Ok((pc, load_addr, payload_bytes))
}

/// No failure marker anywhere, and each progress marker exactly once, in order.
fn require_runtime_markers(stdout: &str) -> Result<()> {
    for marker in FAILURE_MARKERS {
        ensure!(
            !stdout.contains(marker),
            "headless output contains failure marker {marker:?}"
        );
    }
    let mut cursor = 0;
    for marker in MARKERS {
        ensure!(
            stdout.matches(marker).count() == 1,
            "headless output does not contain exactly one {marker:?}"
        );
        let Some(found) = stdout[cursor..].find(marker) else {
            bail!("headless marker is absent or out of order: {marker:?}");
        };
        cursor += found + marker.len();
    }
    Ok(())
}

/// What one replay observed, plus the six logs it wrote.
#[derive(Debug, Clone)]
struct Replay {
    stdout_core: String,
    tick: u64,
    cycles: u64,
    pc_final: u64,
    route_ticks: u64,
    pad_polls: u64,
    vram_fnv: String,
    display_fnv: String,
    display_width: u64,
    display_height: u64,
    /// The logs in `LOG_KINDS` order.
    logs: [PathBuf; 6],
}

impl Replay {
    /// The fields two replays must agree on, in `DETERMINISTIC_FIELDS` order,
    /// each as the text the mismatch message shows.
    fn deterministic(&self) -> [String; 9] {
        [
            self.tick.to_string(),
            self.cycles.to_string(),
            self.pc_final.to_string(),
            self.route_ticks.to_string(),
            self.pad_polls.to_string(),
            format!("{:?}", self.vram_fnv),
            format!("{:?}", self.display_fnv),
            self.display_width.to_string(),
            self.display_height.to_string(),
        ]
    }
}

/// Run the frontend once, headless, and collect what it printed and logged.
/// stdout and stderr share one file so their interleaving is kept, which is
/// what Python's single merged pipe gave.
fn run_once(frontend: &Path, cue: &Path, root: &Path, name: &str) -> Result<Replay> {
    static TICK: OnceLock<Regex> = OnceLock::new();
    static ROUTE: OnceLock<Regex> = OnceLock::new();
    static VRAM: OnceLock<Regex> = OnceLock::new();
    static DISPLAY: OnceLock<Regex> = OnceLock::new();
    let run_root = root.join(name);
    fs::create_dir(&run_root)?;
    let logs = [
        run_root.join("route.csv"),
        run_root.join("cd.csv"),
        run_root.join("gpu.csv"),
        run_root.join("pc.csv"),
        run_root.join("pc-callsite.csv"),
        run_root.join("pc-window.csv"),
    ];
    let output_path = root.join(format!("{name}.out"));
    let output = fs::File::create(&output_path)?;
    let status = Command::new(frontend)
        .args(["launch", "--embedded-playtest", "--path"])
        .arg(cue)
        .args(["--steps", STEPS, "--press", PRESS_ROUTE, "--route-log"])
        .arg(&logs[0])
        .arg("--cd-command-log")
        .arg(&logs[1])
        .arg("--gpu-frame-stats-log")
        .arg(&logs[2])
        .arg("--pc-sample-log")
        .arg(&logs[3])
        .arg("--pc-sample-callsite-log")
        .arg(&logs[4])
        .arg("--pc-sample-window-log")
        .arg(&logs[5])
        .args([
            "--pc-sample-window-ticks",
            "300",
            "--pc-sample-instructions",
            "16384",
            "--dump-hash",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone()?))
        .stderr(Stdio::from(output))
        .status()
        .map_err(|e| Error(format!("cannot run {}: {e}", frontend.display())))?;
    let stdout = String::from_utf8_lossy(&fs::read(&output_path)?).into_owned();
    ensure!(status.success(), "headless replay {name} failed:\n{stdout}");
    require_runtime_markers(&stdout)?;
    let tick = pattern(&TICK, r"tick=(\d+)\s+cycles=(\d+)\s+pc=(0x[0-9a-f]+)").captures(&stdout);
    let route = pattern(&ROUTE, r"route-ticks=(\d+)\s+port1-polls=(\d+)").captures(&stdout);
    let vram = pattern(&VRAM, r"vram_fnv1a_64=(0x[0-9a-f]+)").captures(&stdout);
    let display = pattern(
        &DISPLAY,
        r"display_fnv1a_64=(0x[0-9a-f]+)\s+w=(\d+)\s+h=(\d+)",
    )
    .captures(&stdout);
    let (Some(tick), Some(route), Some(vram), Some(display)) = (tick, route, vram, display) else {
        bail!("headless replay {name} did not print all summaries");
    };
    Ok(Replay {
        stdout_core: stdout_core(&stdout),
        tick: number(&tick[1], 10)?,
        cycles: number(&tick[2], 10)?,
        pc_final: number(&tick[3][2..], 16)?,
        route_ticks: number(&route[1], 10)?,
        pad_polls: number(&route[2], 10)?,
        vram_fnv: vram[1].to_string(),
        display_fnv: display[1].to_string(),
        display_width: number(&display[2], 10)?,
        display_height: number(&display[3], 10)?,
        logs,
    })
}

/// What the CD command log shows: Quake's header and payload were
/// seek-and-read at the table LBA, and something read from the relocated
/// image after the loader finished. Returns the command count, the payload
/// sector count and the first runtime read.
fn cd_evidence(
    path: &Path,
    quake_lba: u64,
    payload_bytes: u64,
    image_lba: u64,
    image_sectors: u64,
) -> Result<(usize, u64, i64)> {
    let csv = Csv::read(path)?;
    let mut read_starts: Vec<i64> = Vec::new();
    let mut seek_read_starts: Vec<i64> = Vec::new();
    for (index, row) in csv.rows.iter().enumerate() {
        let Some(lba) = setloc_lba(&csv, row)? else {
            continue;
        };
        // The commands issued after this SetLoc, up to the next one.
        let mut commands: Vec<&str> = Vec::new();
        for following in &csv.rows[index + 1..] {
            let command = csv.cell(following, "command")?.unwrap_or("");
            if command == "0x02" {
                break;
            }
            commands.push(command);
        }
        if commands.contains(&"0x06") {
            read_starts.push(lba);
        }
        if commands.contains(&"0x15") && commands.contains(&"0x06") {
            seek_read_starts.push(lba);
        }
    }
    let wanted = [quake_lba as i64, quake_lba as i64 + 1];
    ensure!(
        wanted.iter().all(|lba| seek_read_starts.contains(lba)),
        "CD log does not seek/read Quake header and payload at {quake_lba}/{}",
        quake_lba + 1
    );
    let payload_sectors = payload_bytes / disc::USER_DATA_BYTES as u64;
    let payload_end = (quake_lba + 1 + payload_sectors) as i64;
    let image_end = (image_lba + image_sectors) as i64;
    let mut runtime_reads: Vec<i64> = read_starts
        .into_iter()
        .filter(|lba| payload_end <= *lba && *lba < image_end)
        .collect();
    runtime_reads.sort_unstable();
    ensure!(
        !runtime_reads.is_empty(),
        "CD log has no post-loader runtime read inside the relocated Quake image"
    );
    Ok((csv.rows.len(), payload_sectors, runtime_reads[0]))
}

/// How many PC samples landed inside the loaded Quake payload.
fn pc_evidence(path: &Path, load_addr: u64, payload_bytes: u64) -> Result<u64> {
    let csv = Csv::read(path)?;
    let mut samples = 0;
    for row in &csv.rows {
        let pc = parse_int(csv.required(row, "pc")?, 16)?;
        if load_addr <= pc && pc < load_addr + payload_bytes {
            samples += parse_int(csv.required(row, "samples")?, 10)?;
        }
    }
    ensure!(
        samples != 0,
        "PC sampler never observed the loaded Quake address range"
    );
    Ok(samples)
}

/// The two files must hash the same; returns the digest.
fn same(first: &Path, second: &Path, label: &str) -> Result<String> {
    let first_hash = sha256_file(first)?;
    let second_hash = sha256_file(second)?;
    ensure!(
        first_hash == second_hash,
        "{label} differs between replays: {first_hash} != {second_hash}"
    );
    Ok(first_hash)
}

/// A replay's frame hashes and size, held against the pins for a pressing
/// with `expected_menu_entries` carousel cards.
fn require_pins(result: &Replay, label: &str, expected_menu_entries: usize) -> Result<()> {
    let Some((expected_vram, expected_display)) = frame_pins(expected_menu_entries) else {
        bail!("no frame pins for a {expected_menu_entries}-entry pressing");
    };
    let checks = [
        ("tick", result.tick.to_string(), EXPECTED_TICK.to_string()),
        (
            "vram_fnv",
            format!("{:?}", result.vram_fnv),
            format!("{expected_vram:?}"),
        ),
        (
            "display_fnv",
            format!("{:?}", result.display_fnv),
            format!("{expected_display:?}"),
        ),
        (
            "display_width",
            result.display_width.to_string(),
            EXPECTED_DISPLAY.0.to_string(),
        ),
        (
            "display_height",
            result.display_height.to_string(),
            EXPECTED_DISPLAY.1.to_string(),
        ),
    ];
    for (key, found, expected) in checks {
        ensure!(found == expected, "{label} {key} {found} != {expected}");
    }
    Ok(())
}

/// Two replays of one disc have to agree on everything they observed.
fn require_identical_replays(first: &Replay, second: &Replay) -> Result<()> {
    ensure!(
        first.stdout_core == second.stdout_core,
        "programmatic stdout differs between replays"
    );
    let (a, b) = (first.deterministic(), second.deterministic());
    for (index, name) in DETERMINISTIC_FIELDS.iter().enumerate() {
        ensure!(
            a[index] == b[index],
            "{name} differs between replays: {} != {}",
            a[index],
            b[index]
        );
    }
    for (index, kind) in LOG_KINDS.iter().enumerate() {
        same(
            &first.logs[index],
            &second.logs[index],
            &kind.replace('_', " "),
        )?;
    }
    Ok(())
}

/// The receipt's `demo_disc_output`, checked against the cue and bin that
/// will be launched. Returns the bin, the Quake LBA, the image's LBA and its
/// sector count.
fn load_receipt(receipt_path: &Path, cue: &Path) -> Result<(Value, PathBuf, u64, u64, u64)> {
    let bytes =
        fs::read(receipt_path).map_err(|e| Error(format!("{}: {e}", receipt_path.display())))?;
    ensure!(
        bytes.is_ascii(),
        "{}: receipt is not ASCII",
        receipt_path.display()
    );
    let receipt: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        receipt.get("variant") == Some(&Value::String("quake-shareware-default".into())),
        "wrong or missing Quake receipt: {}",
        receipt_path.display()
    );
    let output = field(&receipt, "demo_disc_output")?.clone();
    let toc = field(&output, "quake_toc")?;
    let quake_lba = unsigned(field(toc, "exe_lba")?, "exe_lba")?;
    let image_lba = unsigned(field(toc, "image_lba_offset")?, "image_lba_offset")?;
    let image_sectors = unsigned(
        field(&output, "embedded_quake_data_sectors")?,
        "embedded_quake_data_sectors",
    )?;
    let cue_name = cue
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ensure!(
        Some(cue_name.as_str()) == field(&output, "cue_file")?.as_str()
            && Some(sha256_file(cue)?.as_str()) == field(&output, "cue_sha256")?.as_str(),
        "launched cue does not match the receipt"
    );
    let parent = cue.parent().unwrap_or(Path::new("/"));
    let bin_name = field(&output, "bin_file")?
        .as_str()
        .ok_or_else(|| Error("receipt bin_file is not a string".into()))?;
    let image = resolve(&parent.join(bin_name))?;
    ensure!(
        image.parent() == Some(parent) && image.is_file(),
        "receipt BIN must be a regular file beside the cue"
    );
    ensure!(
        Some(sha256_file(&image)?.as_str()) == field(&output, "bin_sha256")?.as_str(),
        "launched BIN does not match the receipt"
    );
    Ok((output, image, quake_lba, image_lba, image_sectors))
}

pub fn run(raw: &[String]) -> Result<i32> {
    static FILE_LINE: OnceLock<Regex> = OnceLock::new();
    let args = Args::parse(
        raw,
        &["frontend", "cue", "receipt", "expected-menu-entries"],
        &[],
    )?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let frontend = resolve(&args.require_path("frontend")?)?;
    let cue = resolve(&args.require_path("cue")?)?;
    let receipt_path = resolve(&args.require_path("receipt")?)?;
    let expected_menu_entries = match args.int("expected-menu-entries")? {
        None => DEFAULT_MENU_ENTRIES,
        Some(count) => usize::try_from(count).map_err(|_| {
            Error(format!(
                "--expected-menu-entries must not be negative, got {count}"
            ))
        })?,
    };

    let (output, image, quake_lba, image_lba, image_sectors) = load_receipt(&receipt_path, &cue)?;
    let cue_text = fs::read(&cue)?;
    ensure!(cue_text.is_ascii(), "{}: cue is not ASCII", cue.display());
    let cue_text = super::verify::universal_newlines(&String::from_utf8_lossy(&cue_text));
    let cue_files: Vec<String> = pattern(&FILE_LINE, r#"(?m)^FILE "([^"]+)" BINARY$"#)
        .captures_iter(&cue_text)
        .map(|c| c[1].to_string())
        .collect();
    let image_name = image
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ensure!(
        cue_files == [image_name.clone()],
        "cue does not point to the receipt BIN beside it"
    );

    let (selected, menu, payload) = quake_menu_entry(&image, quake_lba, expected_menu_entries)?;
    require_payload_identity(&payload, &output)?;
    let (entry_pc, load_addr, payload_bytes) = embedded_exe_evidence(&image, quake_lba)?;

    let scratch = Scratch::new("psoxide-quake-chainload")?;
    let first = run_once(&frontend, &cue, &scratch.0, "first")?;
    let second = run_once(&frontend, &cue, &scratch.0, "second")?;
    let replays = [("first", &first), ("second", &second)];
    for (name, replay) in replays {
        require_pins(replay, name, expected_menu_entries)?;
        ensure!(
            load_addr <= replay.pc_final && replay.pc_final < load_addr + payload_bytes,
            "{name} final PC {:#010x} is outside the Quake payload",
            replay.pc_final
        );
    }
    require_identical_replays(&first, &second)?;

    let mut evidence = Vec::new();
    for (_, replay) in replays {
        evidence.push(cd_evidence(
            &replay.logs[1],
            quake_lba,
            payload_bytes,
            image_lba,
            image_sectors,
        )?);
    }
    let mut samples = Vec::new();
    for (_, replay) in replays {
        samples.push(pc_evidence(&replay.logs[3], load_addr, payload_bytes)?);
    }
    let payload_sectors = evidence[0].1;

    println!("disc: {image_name}");
    println!(
        "menu: {} visible entries, {QUAKE_ENTRY} at {}/{}: {}",
        menu.len(),
        selected + 1,
        menu.len(),
        menu.join(", ")
    );
    println!(
        "payload matches receipt: EXE LBA {}, FNV-1a-32 {}, menu version {}, {image_sectors} embedded sectors",
        payload.exe_lba, payload.payload_fnv1a32, payload.menu_version
    );
    println!("loader: {payload_sectors} payload sectors; entry {entry_pc:#010x}");
    for (index, (name, replay)) in replays.into_iter().enumerate() {
        println!(
            "chain-load {name}: booted, chain-loaded, Quake Start map resident; final PC {:#010x}; \
             relocated runtime read LBA {}; {} CD commands; {} PC samples inside the payload",
            replay.pc_final, evidence[index].2, evidence[index].0, samples[index]
        );
    }
    println!("both replays identical: stdout, summaries and all six logs");
    println!(
        "route ticks: {}; pad polls: {}; cycles: {}",
        first.route_ticks, first.pad_polls, first.cycles
    );
    println!(
        "VRAM/display FNV-1a-64: {} / {}",
        first.vram_fnv, first.display_fnv
    );
    println!("no screenshots, frame dumps, audio dumps, or guest instrumentation used");
    Ok(0)
}

#[cfg(test)]
#[path = "headless_tests.rs"]
mod tests;
