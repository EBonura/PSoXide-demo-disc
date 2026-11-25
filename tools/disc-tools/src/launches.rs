//! `disc-tools launches`: every program on a pressed disc, started from the launcher, timed.
//!
//! For each member of `members.json` (shelf order), one headless run boots the disc, waits for the
//! shelf, walks Right to the member and presses Cross. The emulator's own route log carries the
//! launcher's `LAUNCHER_STATE` phase, the member's VBlank counter, four words of its text and its
//! ready state, so nothing is read from a TTY:
//!
//! * hand-off: the first tick the launcher's phase is 6 (`launcher/src/state.rs`);
//! * main: the first tick after it with the member's four signature words resident and its VBlank
//!   counter started (1..=10), i.e. the member's own `main` runs;
//! * ready: the first tick from main on with the member's ready state (`ready` in members.json,
//!   from the edition: its first playable screen) holding, after the member's own presses (`press`,
//!   ticks counted from main, played in a second run once main is known; the emulator is
//!   deterministic).
//!
//! On a scene disc the CD log must also show the core reading the member's payload from the
//! sector after its header (`exe_lba + 1`) after the hand-off.
//!
//! Verdict per member: PASS when it reached main and its ready state, and, against `--baseline`
//! (the same runs on an earlier disc, written with `--facts-out`), hand-off to main took no more
//! VBlanks than the baseline's and, where both name a ready state, hand-off to ready neither. A
//! member with no ready state fails the gate (`--facts-out` alone measures without judging).
//!
//! Load time on the disc is hand-off to main: the launcher's and the core's part. Main to ready is
//! the game's own boot, which its own harness judges against its installed build; with a ready
//! state on both discs, hand-off to ready is compared too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Map, Value};

use crate::args::Args;
use crate::replay::Csv;
use crate::util::{dumps, read_json, write_atomic, Error, Result};

/// The tick the walk to the member starts: the shelf is up by then on every disc measured (v0.58
/// 168, the scene disc 317 with its splash played).
const WALK_FROM: u64 = 450;
/// Ticks between two presses of the walk.
const PRESS_GAP: u64 = 12;
/// Launcher phases (`launcher/src/state.rs`).
const PHASE_SHELF: u64 = 2;
const PHASE_HAND_OFF: u64 = 6;
/// Instructions per route tick the step cap allows (a frame is about 560k cycles).
const STEPS_PER_TICK: u64 = 450_000;

/// The address of `symbol` in a link map, as members.rs reads it.
fn map_symbol(map: &str, symbol: &str) -> Option<u32> {
    map.lines().find_map(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        (parts.len() == 5 && parts[4] == symbol)
            .then(|| u32::from_str_radix(parts[0], 16).ok())
            .flatten()
    })
}

fn hex(value: &Value) -> Result<u32> {
    let text = value
        .as_str()
        .ok_or_else(|| Error(format!("not an address: {value}")))?;
    u32::from_str_radix(text.trim_start_matches("0x"), 16)
        .map_err(|_| Error(format!("not an address: {text}")))
}

/// A press list `tick:button[:hold],...` with every tick moved by `by`.
fn shifted(presses: &str, by: u64) -> Result<Vec<String>> {
    presses
        .split(',')
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let (tick, rest) = p
                .trim()
                .split_once(':')
                .ok_or_else(|| Error(format!("press {p:?}: want tick:button[:hold]")))?;
            let tick: u64 = tick
                .parse()
                .map_err(|_| Error(format!("press {p:?}: bad tick")))?;
            Ok(format!("{}:{rest}", tick + by))
        })
        .collect()
}

/// One member's watches and conditions, from its members.json record.
struct Member {
    id: String,
    index: u64,
    exe_lba: u64,
    vblank: Option<u32>,
    words: Vec<(u32, u32)>,
    ready: Option<(u32, String, u64)>,
    /// Bits of the ready word compared (8, 16 or 32, the low ones).
    ready_width: u32,
    /// The ready state as a debug-port line the member prints (`tty PREFIX`).
    ready_tty: Option<String>,
    press: String,
}

impl Member {
    fn from_record(record: &Value) -> Result<Member> {
        let id = record["id"].as_str().unwrap_or("?").to_string();
        let words = record["words"]
            .as_array()
            .ok_or_else(|| Error(format!("{id}: no words")))?
            .iter()
            .map(|pair| Ok((hex(&pair[0])?, hex(&pair[1])?)))
            .collect::<Result<Vec<_>>>()?;
        let probe = if record["ready"].is_object() {
            &record["ready"]
        } else {
            &record["title_probe"]
        };
        let ready_tty = probe["tty"].as_str().map(str::to_string);
        let ready = if probe.is_object() && ready_tty.is_none() {
            Some((
                hex(&probe["addr"])?,
                probe["op"].as_str().unwrap_or("eq").to_string(),
                probe["value"].as_u64().unwrap_or(0),
            ))
        } else {
            None
        };
        Ok(Member {
            index: record["index"].as_u64().unwrap_or(0),
            exe_lba: record["exe_lba"].as_u64().unwrap_or(0),
            vblank: if record["vblank"].is_null() {
                None
            } else {
                Some(hex(&record["vblank"])?)
            },
            words,
            ready,
            ready_width: probe["width"].as_u64().unwrap_or(32) as u32,
            ready_tty,
            press: record["press"].as_str().unwrap_or("").to_string(),
            id,
        })
    }

    fn ready_holds(&self, value: u64) -> bool {
        let value = if self.ready_width < 32 {
            value & ((1 << self.ready_width) - 1)
        } else {
            value
        };
        match &self.ready {
            Some((_, op, want)) if op == "ge" => value >= *want,
            Some((_, op, want)) if op == "ne" => value != *want,
            Some((_, _, want)) => value == *want,
            None => false,
        }
    }
}

/// What one run saw.
#[derive(Default)]
struct Seen {
    shelf: Option<u64>,
    hand_off: Option<u64>,
    main: Option<u64>,
    ready: Option<u64>,
    payload_read: Option<u64>,
}

fn column(addr: u32) -> String {
    format!("ram_{addr:08x}")
}

/// Run the emulator once and read its logs.
fn run_once(
    frontend: &Path,
    cue: &Path,
    state: u32,
    member: &Member,
    presses: &[String],
    max_ticks: u64,
    dir: &Path,
) -> Result<Seen> {
    fs::create_dir_all(dir)?;
    let route = dir.join("route.csv");
    let cd = dir.join("cd.csv");
    let mut watches = vec![state];
    watches.extend(member.vblank);
    watches.extend(member.words.iter().map(|(a, _)| *a));
    if let Some((addr, ..)) = &member.ready {
        watches.push(*addr);
    }
    watches.dedup();
    let mut command = Command::new(frontend);
    command
        .arg("launch")
        .arg("--path")
        .arg(cue)
        .arg("--steps")
        .arg((max_ticks * STEPS_PER_TICK).to_string())
        .arg("--press")
        .arg(presses.join(","))
        .arg("--route-log")
        .arg(&route)
        .arg("--cd-command-log")
        .arg(&cd);
    if let Some(prefix) = &member.ready_tty {
        command.arg("--stop-on-tty").arg(prefix);
    }
    for addr in &watches {
        command.arg("--route-watch-u32").arg(format!("{addr:#x}"));
    }
    let output = command.output()?;
    fs::write(dir.join("out.txt"), [&output.stdout[..], &output.stderr[..]].concat())?;
    ensure!(
        route.is_file(),
        "{}: the emulator wrote no route log (see {})",
        member.id,
        dir.join("out.txt").display()
    );
    let log = Csv::read(&route)?;
    let value = |row: &[String], addr: u32| -> Result<u64> {
        Ok(u64::from_str_radix(log.required(row, &column(addr))?, 10).unwrap_or(0))
    };
    let mut seen = Seen::default();
    let mut cycles = Vec::with_capacity(log.rows.len());
    for row in &log.rows {
        let tick: u64 = log.required(row, "route_tick")?.parse().unwrap_or(0);
        cycles.push(log.required(row, "bus_cycles")?.parse::<u64>().unwrap_or(0));
        let phase = value(row, state)?;
        if seen.shelf.is_none() && phase == PHASE_SHELF {
            seen.shelf = Some(tick);
        }
        if seen.hand_off.is_none() && phase == PHASE_HAND_OFF {
            seen.hand_off = Some(tick);
            continue;
        }
        let Some(_) = seen.hand_off else { continue };
        let resident = member
            .words
            .iter()
            .map(|(addr, word)| Ok(value(row, *addr)? == u64::from(*word)))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|ok| ok);
        let started = match member.vblank {
            Some(addr) => (1..=10).contains(&value(row, addr)?),
            None => true,
        };
        if seen.main.is_none() && resident && started {
            seen.main = Some(tick);
        }
        // The ready word comes from its own map, which may be another program of the member (a boot
        // program that switches to the game): the boot program's words need not be resident.
        if seen.main.is_some() && seen.ready.is_none() {
            if let Some((addr, ..)) = &member.ready {
                if member.ready_holds(value(row, *addr)?) {
                    seen.ready = Some(tick);
                }
            }
        }
    }
    // A debug-port ready state: the run stopped on the member's line, at its last route tick.
    if member.ready_tty.is_some() && seen.main.is_some() {
        let text = String::from_utf8_lossy(&fs::read(dir.join("out.txt"))?).into_owned();
        if text.contains("stopped-at=") {
            seen.ready = log
                .rows
                .last()
                .and_then(|row| log.required(row, "route_tick").ok())
                .and_then(|t| t.parse().ok());
        }
    }
    // The core reads the payload from the sector after the header.
    if let (Some(hand_off), Ok(cd_log)) = (seen.hand_off, Csv::read(&cd)) {
        for row in &cd_log.rows {
            if cd_log.required(row, "command")? != "0x02" {
                continue;
            }
            let cycle: u64 = cd_log.required(row, "cycle")?.parse().unwrap_or(0);
            let tick = cycles.partition_point(|&c| c < cycle) as u64;
            let bcd = |t: &str| u64::from_str_radix(t, 16).map(|v| (v >> 4) * 10 + (v & 15));
            let msf: Vec<u64> = cd_log
                .required(row, "params")?
                .split_whitespace()
                .filter_map(|t| bcd(t).ok())
                .collect();
            if let [m, s, f] = msf[..] {
                let lba = (m * 60 + s) * 75 + f;
                if tick >= hand_off && lba >= 150 && lba - 150 == member.exe_lba + 1 {
                    seen.payload_read = Some(tick);
                    break;
                }
            }
        }
    }
    Ok(seen)
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(
        raw,
        &[
            "frontend",
            "cue",
            "members",
            "launcher-map",
            "out",
            "baseline",
            "facts-out",
            "max-ticks",
            "only",
        ],
        &["chain"],
    )?;
    let frontend = args.require_path("frontend")?;
    let cue = args.require_path("cue")?;
    let out = args.require_path("out")?;
    let records = read_json(&args.require_path("members")?)?;
    let launcher_map = fs::read_to_string(args.require_path("launcher-map")?)?;
    let state = map_symbol(&launcher_map, "LAUNCHER_STATE")
        .ok_or_else(|| Error("LAUNCHER_STATE is not in the launcher map".into()))?;
    let max_ticks = args.int("max-ticks")?.unwrap_or(4000) as u64;
    let baseline = args.path("baseline").map(|p| read_json(&p)).transpose()?;
    // A chain-loaded pressing (v0.59 and before) loads with the blob, not the core: no payload read
    // check, measured for the baseline only.
    let chain = args.has("chain");
    let only = args.all("only");
    let judging = baseline.is_some() || args.path("facts-out").is_none();
    let mut facts = Map::new();
    let mut failures = 0;
    for record in records
        .as_array()
        .ok_or_else(|| Error("members.json: not a list".into()))?
    {
        let member = Member::from_record(record)?;
        if !only.is_empty() && !only.contains(&member.id.as_str()) {
            continue;
        }
        let mut walk: Vec<String> = (0..member.index)
            .map(|k| format!("{}:right", WALK_FROM + k * PRESS_GAP))
            .collect();
        walk.push(format!("{}:cross", WALK_FROM + (member.index + 1) * PRESS_GAP));
        let slug: String = member
            .id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect();
        let dir: PathBuf = out.join(&slug);
        let mut seen = run_once(&frontend, &cue, state, &member, &walk, max_ticks, &dir)?;
        if let (Some(main), false) = (seen.main, member.press.is_empty()) {
            let mut presses = walk.clone();
            presses.extend(shifted(&member.press, main)?);
            seen = run_once(&frontend, &cue, state, &member, &presses, max_ticks, &dir)?;
        }
        let mut problems: Vec<String> = Vec::new();
        if seen.shelf.is_none_or(|t| t > WALK_FROM) {
            problems.push(format!("the shelf was not up by tick {WALK_FROM}"));
        }
        let Some(hand_off) = seen.hand_off else {
            problems.push("the launcher never handed off".into());
            println!("{}: FAIL ({})", member.id, problems.join("; "));
            failures += 1;
            continue;
        };
        if !chain && seen.payload_read.is_none() {
            problems.push(format!(
                "no read of the payload at LBA {} after the hand-off",
                member.exe_lba + 1
            ));
        }
        let to_main = seen.main.map(|t| t - hand_off);
        let to_ready = seen.ready.map(|t| t - hand_off);
        match to_main {
            None => problems.push(format!("its main never ran within {max_ticks} ticks")),
            Some(_) if member.ready.is_none() && member.ready_tty.is_none() => {
                problems.push("no ready state (its first playable screen) is named".into())
            }
            Some(_) if to_ready.is_none() => {
                problems.push(format!("its ready state never held within {max_ticks} ticks"))
            }
            Some(_) => {}
        }
        let base = baseline.as_ref().map(|b| &b["launches"][&member.id]);
        let mut compare = String::new();
        if let Some(base) = base {
            match (base["to_main"].as_u64(), to_main) {
                (Some(was), Some(now)) => {
                    compare.push_str(&format!(" to main {now} vs {was}"));
                    if now > was {
                        problems.push(format!("hand-off to main {now} VBlanks, baseline {was}"));
                    }
                }
                // A game that was not on the baseline disc has no launch to compare with.
                (None, _) => compare.push_str(" (not on the baseline disc: not compared)"),
                _ => {}
            }
            if let (Some(was), Some(now)) = (base["to_ready"].as_u64(), to_ready) {
                compare.push_str(&format!(", to ready {now} vs {was}"));
                if now > was {
                    problems.push(format!("hand-off to ready {now} VBlanks, baseline {was}"));
                }
            }
        }
        facts.insert(
            member.id.clone(),
            json!({
                "hand_off": hand_off, "main": seen.main, "ready": seen.ready,
                "to_main": to_main, "to_ready": to_ready, "payload_read": seen.payload_read,
                "problems": problems,
            }),
        );
        if judging && !problems.is_empty() {
            failures += 1;
            println!("{}: FAIL ({}){compare}", member.id, problems.join("; "));
        } else {
            println!(
                "{}: {} hand-off {hand_off}, to main {}, to ready {}{compare}",
                member.id,
                if judging { "PASS" } else { "measured" },
                to_main.map_or("-".into(), |v| v.to_string()),
                to_ready.map_or("-".into(), |v| v.to_string())
            );
        }
    }
    let document = json!({"disc": cue.display().to_string(), "launches": facts});
    write_atomic(&out.join("launches.json"), &(dumps(&document, 1, false) + "\n"))?;
    if let Some(path) = args.path("facts-out") {
        write_atomic(&path, &(dumps(&document, 1, false) + "\n"))?;
    }
    if judging {
        println!(
            "launches: {}",
            if failures == 0 { "PASS" } else { "FAIL" }
        );
    }
    Ok(i32::from(failures != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_move_with_main() {
        assert_eq!(
            shifted("30:start,90:cross:8", 700).unwrap(),
            vec!["730:start", "790:cross:8"]
        );
        assert!(shifted("start", 0).is_err());
    }

    #[test]
    fn map_symbols_are_read_by_name() {
        let map = "80031364 80031364       18     1                 LAUNCHER_STATE\n";
        assert_eq!(map_symbol(map, "LAUNCHER_STATE"), Some(0x8003_1364));
        assert_eq!(map_symbol(map, "OTHER"), None);
    }
}
