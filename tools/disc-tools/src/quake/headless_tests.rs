//! Ports of the original `HeadlessChainloadTests`: pure-function tests over
//! synthetic tables and CSV logs. No emulator runs here.

use std::fs;

use serde_json::{json, Value};

use super::*;

const QUAKE_LBA: u64 = 50;
const PAYLOAD_FNV: u32 = 0x1234_5678;
const MENU_VERSION: &str = "q2d26f9e";

type Entries = Vec<(String, u32, u32)>;

/// The standard pressing's locked shape: one hidden, seven, Quake. Eight
/// visible programs plus the launcher's CREDITS card is nine, and the
/// headless route's two RIGHT presses still land on Quake.
fn default_entries() -> Entries {
    let mut entries = vec![("CORTEX IGNITION".to_string(), 30, disc::FLAG_HIDDEN)];
    entries.extend((0..7).map(|index| (format!("PROGRAM {index}"), 31 + index, 0)));
    entries.push((QUAKE_ENTRY.to_string(), QUAKE_LBA as u32, 0));
    entries
}

/// A raw disc with `entries` in its table and a PS-X EXE header at Quake's LBA.
fn make_disc_image(root: &Path, entries: &Entries) -> PathBuf {
    let sector = disc::SECTOR_BYTES as usize;
    let user = disc::USER_DATA_BYTES;
    let mut image = vec![0u8; (QUAKE_LBA as usize + 2) * sector];
    let mut toc = vec![0u8; disc::TOC_SECTORS as usize * user];
    toc[..8].copy_from_slice(disc::TOC_MAGIC);
    toc[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
    for (index, (name, lba, flags)) in entries.iter().enumerate() {
        let at = TOC_HEADER_BYTES + index * TOC_ENTRY_BYTES;
        toc[at..at + name.len()].copy_from_slice(name.as_bytes());
        toc[at + TOC_NAME_BYTES..at + TOC_NAME_BYTES + 4].copy_from_slice(&lba.to_le_bytes());
        toc[at + TOC_FLAGS_AT..at + TOC_FLAGS_AT + 4].copy_from_slice(&flags.to_le_bytes());
        if name != QUAKE_ENTRY {
            continue;
        }
        toc[at + TOC_PAYLOAD_FNV_AT..at + TOC_PAYLOAD_FNV_AT + 4]
            .copy_from_slice(&PAYLOAD_FNV.to_le_bytes());
        toc[at + TOC_VERSION_AT..at + TOC_VERSION_AT + MENU_VERSION.len()]
            .copy_from_slice(MENU_VERSION.as_bytes());
    }
    for index in 0..disc::TOC_SECTORS as usize {
        let target = (disc::TOC_LBA as usize + index) * sector + disc::USER_DATA_AT as usize;
        image[target..target + user].copy_from_slice(&toc[index * user..(index + 1) * user]);
    }
    let mut header = vec![0u8; user];
    header[..8].copy_from_slice(disc::PSX_EXE_MAGIC);
    header[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    header[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    header[0x1C..0x20].copy_from_slice(&4_096u32.to_le_bytes());
    let target = QUAKE_LBA as usize * sector + disc::USER_DATA_AT as usize;
    image[target..target + user].copy_from_slice(&header);
    let path = root.join("disc.bin");
    fs::write(&path, image).unwrap();
    path
}

fn expected_payload() -> Payload {
    Payload {
        exe_lba: QUAKE_LBA,
        payload_fnv1a32: format!("0x{PAYLOAD_FNV:08x}"),
        menu_version: MENU_VERSION.to_string(),
    }
}

fn receipt_output() -> Value {
    json!({
        "quake_toc": {
            "exe_lba": QUAKE_LBA,
            "payload_fnv1a32": format!("0x{PAYLOAD_FNV:08x}"),
            "menu_version": MENU_VERSION,
        },
        "embedded_quake_data_sectors": 9_465,
        "embedded_quake_matches_input_except_msf": true,
    })
}

fn message<T>(result: Result<T>) -> String {
    match result {
        Ok(_) => panic!("expected a failure"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn route_selects_visible_quake_on_the_default_carousel() {
    let dir = tempfile::tempdir().unwrap();
    let image = make_disc_image(dir.path(), &default_entries());
    let (selected, menu, payload) =
        quake_menu_entry(&image, QUAKE_LBA, DEFAULT_MENU_ENTRIES).unwrap();
    assert_eq!(menu.len(), DEFAULT_MENU_ENTRIES);
    assert_eq!(selected + 1, DEFAULT_MENU_ENTRIES - 1);
    assert_eq!(menu[selected], QUAKE_ENTRY);
    assert_eq!(menu[0], "PROGRAM 0");
    assert!(!menu.contains(&"CORTEX IGNITION".to_string()));
    assert_eq!(menu.last().unwrap(), "CREDITS");
    assert_eq!(payload, expected_payload());
    assert_eq!(
        embedded_exe_evidence(&image, QUAKE_LBA).unwrap(),
        (0x8001_0000, 0x8001_0000, 4_096)
    );
}

#[test]
fn route_selects_visible_quake_on_the_half_life_carousel() {
    let default = default_entries();
    let mut entries: Entries = vec![
        (default[0].0.clone(), default[0].1, 0),
        ("HALF-LIFE".to_string(), 41, 0),
    ];
    entries.extend(default[1..].iter().cloned());
    let dir = tempfile::tempdir().unwrap();
    let image = make_disc_image(dir.path(), &entries);
    let (selected, menu, _) = quake_menu_entry(&image, QUAKE_LBA, 11).unwrap();
    assert_eq!(menu.len(), 11);
    assert_eq!(selected + 1, 10);
    assert_eq!(menu[..2], ["CORTEX IGNITION", "HALF-LIFE"]);
    assert_eq!(menu[selected], QUAKE_ENTRY);
    assert_eq!(menu.last().unwrap(), "CREDITS");
}

#[test]
fn a_disc_without_a_visible_quake_entry_fails() {
    let mut absent: Entries = default_entries()
        .into_iter()
        .filter(|e| e.0 != QUAKE_ENTRY)
        .collect();
    absent.push(("TENTH".to_string(), 41, 0));
    let hidden: Entries = default_entries()
        .into_iter()
        .map(|(name, lba, flags)| {
            if name == QUAKE_ENTRY {
                (name, lba, disc::FLAG_HIDDEN)
            } else {
                (name, lba, flags)
            }
        })
        .collect();
    let mut crowded = default_entries();
    crowded.push(("EXTRA".to_string(), 41, 0));
    let cases = [
        ("absent", absent, "no QUAKE SHAREWARE entry"),
        ("hidden", hidden, "hidden from the carousel"),
        ("wrong entry count", crowded, "visible entries, expected"),
    ];
    for (label, entries, error) in cases {
        let dir = tempfile::tempdir().unwrap();
        let image = make_disc_image(dir.path(), &entries);
        let text = message(quake_menu_entry(&image, QUAKE_LBA, DEFAULT_MENU_ENTRIES));
        assert!(text.contains(error), "{label}: {text}");
    }
}

#[test]
fn a_route_that_lands_on_another_card_fails() {
    // Quake sits first among the visible cards, so two RIGHT presses from
    // there select a filler card, and the route check has to notice.
    let mut entries = default_entries();
    let quake = entries.pop().unwrap();
    entries.insert(1, quake);
    let dir = tempfile::tempdir().unwrap();
    let image = make_disc_image(dir.path(), &entries);
    let text = message(quake_menu_entry(&image, QUAKE_LBA, DEFAULT_MENU_ENTRIES));
    assert!(text.contains("menu route selects"), "{text}");
}

#[test]
fn payload_identity_is_held_against_the_receipt() {
    let payload = expected_payload();
    require_payload_identity(&payload, &receipt_output()).unwrap();
    for (name, drifted) in [
        ("exe_lba", json!(41)),
        ("payload_fnv1a32", json!("0xdeadbeef")),
        ("menu_version", json!("q0000000")),
    ] {
        let mut output = receipt_output();
        output["quake_toc"][name] = drifted;
        let text = message(require_payload_identity(&payload, &output));
        assert!(text.contains(name), "{name}: {text}");
    }
    let mut output = receipt_output();
    output["embedded_quake_matches_input_except_msf"] = json!(false);
    assert!(message(require_payload_identity(&payload, &output)).contains("embedded Quake"));
    let mut output = receipt_output();
    output["embedded_quake_data_sectors"] = json!(0);
    assert!(message(require_payload_identity(&payload, &output))
        .contains("no embedded Quake data sectors"));
}

/// A replay whose logs are real files holding `kind`, so they hash.
fn replay_in(dir: &Path) -> Replay {
    let kinds = LOG_KINDS.map(|kind| {
        let path = dir.join(format!("{kind}.csv"));
        fs::write(&path, format!("{kind}\n")).unwrap();
        path
    });
    Replay {
        stdout_core: "same".into(),
        tick: 1,
        cycles: 1,
        pc_final: 1,
        route_ticks: 1,
        pad_polls: 1,
        vram_fnv: "1".into(),
        display_fnv: "1".into(),
        display_width: 1,
        display_height: 1,
        logs: kinds,
    }
}

#[test]
fn replays_must_agree_on_everything_they_observed() {
    let dir = tempfile::tempdir().unwrap();
    let base = replay_in(dir.path());
    require_identical_replays(&base, &base.clone()).unwrap();

    // One drift per deterministic field, each named in the message.
    type Drift = fn(&mut Replay);
    let drifts: [(&str, Drift); 9] = [
        ("tick", |r| r.tick = 2),
        ("cycles", |r| r.cycles = 2),
        ("pc_final", |r| r.pc_final = 2),
        ("route_ticks", |r| r.route_ticks = 2),
        ("pad_polls", |r| r.pad_polls = 2),
        ("vram_fnv", |r| r.vram_fnv = "2".into()),
        ("display_fnv", |r| r.display_fnv = "2".into()),
        ("display_width", |r| r.display_width = 2),
        ("display_height", |r| r.display_height = 2),
    ];
    assert_eq!(drifts.map(|(name, _)| name), DETERMINISTIC_FIELDS);
    for (name, drift) in drifts {
        let mut drifted = base.clone();
        drift(&mut drifted);
        let text = message(require_identical_replays(&base, &drifted));
        assert!(text.contains(name), "{name}: {text}");
    }

    let mut other = base.clone();
    other.stdout_core = "different".into();
    assert!(message(require_identical_replays(&base, &other)).contains("stdout"));

    // A log that differs by one byte must be caught for every kind.
    for (index, kind) in LOG_KINDS.iter().enumerate() {
        let elsewhere = dir.path().join(format!("second-{kind}.csv"));
        fs::write(&elsewhere, "elsewhere\n").unwrap();
        let mut drifted = base.clone();
        drifted.logs[index] = elsewhere;
        let text = message(require_identical_replays(&base, &drifted));
        assert!(text.contains(&kind.replace('_', " ")), "{kind}: {text}");
    }
}

#[test]
fn only_build_independent_values_are_pinned() {
    let source = include_str!("headless.rs");
    // These moved with the launcher binary, which changes on every commit
    // here, so they are held to run-to-run equality and nothing more.
    for pin in [
        "EXPECTED_CYCLES",
        "EXPECTED_PC ",
        "EXPECTED_ROUTE_TICKS",
        "EXPECTED_PAD_POLLS",
        "EXPECTED_CD_COMMANDS",
        "EXPECTED_LOG_SHA256",
    ] {
        assert!(!source.contains(pin), "{pin} must not be pinned");
    }
    for name in ["cycles", "route_ticks", "pad_polls"] {
        assert!(
            DETERMINISTIC_FIELDS.contains(&name),
            "{name} must be held to run-to-run equality"
        );
    }
}

#[test]
fn each_pressing_has_its_own_visible_frame_pins() {
    // 10 is the public pressing since Cortex joined its carousel on
    // 2026-09-03, 11 the private Half-Life pressing; the checker's own
    // default layout keeps its pins for the older public image.
    let sizes: Vec<usize> = EXPECTED_FRAME_FNV_BY_MENU_ENTRIES
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(sizes, [DEFAULT_MENU_ENTRIES, 10, 11]);
    let mut result = replay_in(tempfile::tempdir().unwrap().path());
    for (menu_entries, (vram, display)) in EXPECTED_FRAME_FNV_BY_MENU_ENTRIES {
        result.tick = EXPECTED_TICK;
        result.vram_fnv = vram.into();
        result.display_fnv = display.into();
        result.display_width = EXPECTED_DISPLAY.0;
        result.display_height = EXPECTED_DISPLAY.1;
        require_pins(&result, "fixture", menu_entries).unwrap();
    }
    assert!(message(require_pins(&result, "fixture", 13)).contains("no frame pins"));
    // And a real drift is named.
    result.vram_fnv = "0x0".into();
    assert!(message(require_pins(&result, "second", 11)).contains("second vram_fnv"));
}

#[test]
fn cd_evidence_requires_header_and_payload_read_sequences() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("cd.csv");
    fs::write(
        &log,
        "cycle,command,param_len,params\n\
         1,0x02,3,00 02 40\n\
         2,0x15,0,\n\
         3,0x06,0,\n\
         4,0x02,3,00 02 41\n\
         5,0x15,0,\n\
         6,0x06,0,\n\
         7,0x02,3,00 02 45\n\
         8,0x15,0,\n\
         9,0x06,0,\n",
    )
    .unwrap();
    assert_eq!(cd_evidence(&log, 40, 4_096, 20, 100).unwrap(), (9, 2, 45));
    fs::write(
        &log,
        "cycle,command,param_len,params\n1,0x02,3,00 02 40\n2,0x15,0,\n3,0x06,0,\n",
    )
    .unwrap();
    assert!(
        message(cd_evidence(&log, 40, 4_096, 20, 100)).contains("does not seek/read Quake header")
    );
}

#[test]
fn cd_evidence_needs_a_runtime_read_inside_the_relocated_image() {
    // Header and payload were read, but nothing after them: the loader
    // finished and Quake never touched its own image.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("cd.csv");
    fs::write(
        &log,
        "cycle,command,param_len,params\n\
         1,0x02,3,00 02 40\n2,0x15,0,\n3,0x06,0,\n\
         4,0x02,3,00 02 41\n5,0x15,0,\n6,0x06,0,\n",
    )
    .unwrap();
    assert!(message(cd_evidence(&log, 40, 4_096, 20, 100)).contains("no post-loader runtime read"));
}

#[test]
fn cd_log_helpers_follow_the_python_readers() {
    assert_eq!(bcd("40").unwrap(), 40);
    assert_eq!(bcd("0x09").unwrap(), 9);
    assert!(message(bcd("0a")).contains("invalid BCD byte 0a"));
    assert_eq!(parse_int("0x8001a000", 16).unwrap(), 0x8001_a000);
    assert_eq!(parse_int(" 12 ", 10).unwrap(), 12);
    assert!(parse_int("zz", 16).is_err());

    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("pc.csv");
    fs::write(
        &log,
        "pc,samples\n0x80010000,5\n0x80010800,7\n0x90000000,100\n",
    )
    .unwrap();
    assert_eq!(pc_evidence(&log, 0x8001_0000, 4_096).unwrap(), 12);
    assert!(message(pc_evidence(&log, 0x8002_0000, 4_096)).contains("never observed"));
    fs::write(&log, "pc\n0x80010000\n").unwrap();
    assert!(message(pc_evidence(&log, 0x8001_0000, 4_096)).contains("samples"));
}

#[test]
fn csv_reader_matches_dictreader_on_quotes_and_blank_lines() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("x.csv");
    fs::write(&log, "a,b\r\n\r\n\"x,1\",\"say \"\"hi\"\"\"\r\nlast,\n").unwrap();
    let csv = Csv::read(&log).unwrap();
    assert_eq!(csv.header, ["a", "b"]);
    assert_eq!(csv.rows, [vec!["x,1", "say \"hi\""], vec!["last", ""]]);
    assert_eq!(csv.cell(&csv.rows[1], "b").unwrap(), Some(""));
    assert!(csv.cell(&csv.rows[0], "missing").is_err());
}

#[test]
fn runtime_markers_are_ordered_and_fail_closed() {
    let output = MARKERS.join("\n");
    require_runtime_markers(&output).unwrap();
    let mut reversed = MARKERS;
    reversed.reverse();
    assert!(message(require_runtime_markers(&reversed.join("\n"))).contains("out of order"));
    let failed = format!("{output}\nquake-psx: Rust initial level load failed");
    assert!(message(require_runtime_markers(&failed)).contains("failure marker"));
    // A marker printed twice is as bad as one missing.
    let twice = format!("{output}\n{}", MARKERS[0]);
    assert!(message(require_runtime_markers(&twice)).contains("exactly one"));
    assert!(message(require_runtime_markers("nothing")).contains("exactly one"));
}

#[test]
fn stdout_core_drops_cli_lines_and_splits_like_python() {
    assert_eq!(
        stdout_core("[cli] a\nkeep\r\n[cli] b\nalso\rlast"),
        "keep\nalso\nlast"
    );
    assert_eq!(splitlines("a\n\nb\n"), ["a", "", "b"]);
    assert_eq!(splitlines(""), Vec::<&str>::new());
    assert_eq!(route_button_count("right"), 2);
    assert_eq!(route_button_count("cross"), 1);
}

#[test]
fn headless_gate_cannot_create_media_dumps() {
    let source = include_str!("headless.rs");
    assert!(source.contains("\"--embedded-playtest\""));
    for flag in ["--dump-display", "--dump-vram", "--dump-hw", "--dump-audio"] {
        assert!(
            !source.contains(flag),
            "{flag} must not be passed to the frontend"
        );
    }
}
