//! Check Celeste return and credits navigation through the actual demo disc.
//!
//! Boots the supplied pressing, reads its carousel order from the table,
//! launches each cart, and checks pause-quit and Select+Start returns with
//! both digital and analog controllers. Keeps logs, screenshots and an
//! identity file naming the exact disc and emulator used. A missing
//! checkpoint, unexpected screen, early exit or timeout fails the route.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::args::Args;
use crate::programs::{cue_bin, menu_presses, read_rows, FAILURE_MARKERS};
use crate::replay::Ppm;
use crate::util::{dumps, resolve, sha256_file, Error, Result};

/// What a checkpoint screenshot shows: the screen kind and, for the Celeste
/// menu, which cart (0 or 1) is highlighted.
type Screen = (&'static str, Option<i64>);
/// Tick of the screenshot, the screen expected there and, for the menu, the
/// highlighted cart.
type Check = (i64, &'static str, Option<i64>);

const MENU_PROBE: (usize, usize, usize, usize) = (156, 202, 200, 226);
const MENU_ACCENT: [u8; 3] = [222, 206, 115];
const COVERS: [(usize, usize, usize, usize); 2] = [(52, 68, 132, 148), (188, 68, 268, 148)];
const CREDITS_BACKGROUND: [u8; 3] = [8, 8, 24];
const PAUSE_BORDER: [u8; 3] = [255, 247, 239];

fn count_in(image: &Ppm, (x0, y0, x1, y1): (usize, usize, usize, usize), colour: [u8; 3]) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| image.pixel(x, y) == colour)
        .count()
}

fn channel_sum(image: &Ppm, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> u64 {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .map(|(x, y)| image.pixel(x, y).iter().map(|&c| u64::from(c)).sum::<u64>())
        .sum()
}

/// Classify a 320x240 frame: carousel menu, credits, pause screen or unknown.
fn classify(image: &Ppm) -> Screen {
    if (image.width, image.height) != (320, 240) {
        return ("unknown", None);
    }
    if count_in(image, MENU_PROBE, MENU_ACCENT) >= 150 {
        let covers = COVERS.map(|boxed| channel_sum(image, boxed));
        return ("menu", Some(i64::from(covers[1] > covers[0])));
    }
    if count_in(image, (0, 0, 320, 240), CREDITS_BACKGROUND) >= 320 * 240 * 3 / 10 {
        return ("credits", None);
    }
    let border = (60..180)
        .filter(|&y| image.pixel(56, y) == PAUSE_BORDER && image.pixel(262, y) == PAUSE_BORDER)
        .count();
    if border >= 100 {
        return ("pause", None);
    }
    ("unknown", None)
}

fn screen(path: &Path) -> Screen {
    match fs::read(path).ok().as_deref().and_then(Ppm::parse) {
        Some(image) => classify(&image),
        None => ("unknown", None),
    }
}

fn save_png(image: &Ppm, path: &Path) -> Result<()> {
    let file = fs::File::create(path)?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file),
        image.width as u32,
        image.height as u32,
    );
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    writer
        .write_image_data(&image.pixels)
        .map_err(|e| Error(format!("{}: {e}", path.display())))?;
    Ok(())
}

/// Entering the first cart title and dismissing it: the launch presses shared
/// by every route.
fn entry_events(prefix: &[String], launch: i64, cart: i64) -> Vec<String> {
    let mut events = prefix.to_vec();
    if cart != 0 {
        events.push(format!("{}:right:8", launch + 1500));
    }
    events.extend([1600, 2200, 2800].map(|t| format!("{}:cross:12", launch + t + cart * 100)));
    events
}

/// Pause-menu quit or Select+Start chord back to the collection menu, then
/// credits and back.
fn route(prefix: &[String], launch: i64, cart: i64, chord: bool) -> (Vec<String>, Vec<Check>) {
    // Times are relative to the outer carousel's launch, not the standalone
    // collection's boot. Leave room for the CD loader and both title fades.
    let start = launch + 3200 + cart * 100;
    let mut events = entry_events(prefix, launch, cart);
    let mut checks: Vec<Check> = vec![(launch + 1400, "menu", Some(0))];
    if chord {
        events.extend([
            format!("{start}:select:30"),
            format!("{}:start:26", start + 4),
        ]);
    } else {
        events.extend([
            format!("{start}:start:8"),
            format!("{}:up:8", start + 160),
            format!("{}:cross:8", start + 320),
        ]);
        checks.push((start + 100, "pause", None));
    }
    checks.push((start + 500, "menu", Some(cart)));
    events.extend([
        format!("{}:select:8", start + 700),
        format!("{}:cross:8", start + 1100),
    ]);
    checks.extend([
        (start + 900, "credits", None),
        (start + 1300, "menu", Some(cart)),
    ]);
    (events, checks)
}

fn stress_route(prefix: &[String], launch: i64, cart: i64) -> (Vec<String>, Vec<Check>) {
    // First quit after a longer gameplay interval, holding Cross across the
    // menu transition. Then relaunch and quit with Select still held, and
    // finally visit the credits while holding Select and exit with Cross.
    let mut events = entry_events(prefix, launch, cart);
    let mut checks: Vec<Check> = vec![(launch + 1400, "menu", Some(0))];
    let start = launch + 9000;
    events.extend([
        format!("{start}:start:8"),
        format!("{}:up:8", start + 160),
        format!("{}:cross:400", start + 320),
    ]);
    checks.extend([
        (start + 100, "pause", None),
        (start + 600, "menu", Some(cart)),
        (start + 900, "menu", Some(cart)),
    ]);
    events.extend([1000, 1600, 2200].map(|t| format!("{}:cross:12", start + t)));
    events.extend([
        format!("{}:select:700", start + 2600),
        format!("{}:start:200", start + 2604),
    ]);
    checks.extend([
        (start + 3000, "menu", Some(cart)),
        (start + 3500, "menu", Some(cart)),
    ]);
    events.extend([
        format!("{}:select:700", start + 3700),
        format!("{}:cross:200", start + 4100),
    ]);
    checks.extend([
        (start + 3900, "credits", None),
        (start + 4300, "menu", Some(cart)),
        (start + 4600, "menu", Some(cart)),
    ]);
    // A third entry and pause-quit guards against stale global input state.
    events.extend([4800, 5400, 6000].map(|t| format!("{}:cross:12", start + t)));
    events.extend([
        format!("{}:start:8", start + 6400),
        format!("{}:up:8", start + 6560),
        format!("{}:cross:8", start + 6720),
    ]);
    checks.extend([
        (start + 6500, "pause", None),
        (start + 7000, "menu", Some(cart)),
    ]);
    (events, checks)
}

fn save_route(prefix: &[String], launch: i64, cart: i64) -> (Vec<String>, Vec<Check>) {
    let mut events = entry_events(prefix, launch, cart);
    let start = launch + 3200 + cart * 100;
    // SFX starts at eight on this fresh memory card. Change it to seven, then
    // wrap up to Quit. Leaving the dirty pause menu must write the card.
    events.extend([
        format!("{start}:start:8"),
        format!("{}:left:8", start + 160),
        format!("{}:up:8", start + 320),
        format!("{}:cross:200", start + 480),
        format!("{}:select:8", start + 1700),
        format!("{}:cross:8", start + 2100),
    ]);
    let checks = vec![
        (launch + 1400, "menu", Some(0)),
        (start + 100, "pause", None),
        (start + 300, "pause", None),
        (start + 1500, "menu", Some(cart)),
        (start + 1900, "credits", None),
        (start + 2300, "menu", Some(cart)),
    ];
    (events, checks)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suite {
    Basic,
    Stress,
    Save,
}

impl Suite {
    fn name(self) -> &'static str {
        match self {
            Suite::Basic => "basic",
            Suite::Stress => "stress",
            Suite::Save => "save",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Case {
    cart: i64,
    chord: bool,
    digital: bool,
    suite: Suite,
}

struct Shared<'a> {
    frontend: &'a Path,
    cue: &'a Path,
    out: &'a Path,
    prefix: &'a [String],
    launch: i64,
}

fn run_case(shared: &Shared, case: Case) -> Result<Value> {
    let Case {
        cart,
        chord,
        digital,
        suite,
    } = case;
    let name = if suite == Suite::Basic {
        format!(
            "c{}-{}-{}",
            cart + 1,
            if chord { "chord" } else { "pause" },
            if digital { "digital" } else { "analog" }
        )
    } else {
        format!("c{}-{}-analog", cart + 1, suite.name())
    };
    let folder = shared.out.join(&name);
    let shots = folder.join("shots");
    fs::create_dir_all(&shots)?;
    let (events, checks) = match suite {
        Suite::Stress => stress_route(shared.prefix, shared.launch, cart),
        Suite::Save => save_route(shared.prefix, shared.launch, cart),
        Suite::Basic => route(shared.prefix, shared.launch, cart, chord),
    };
    let last = checks.iter().map(|check| check.0).max().unwrap_or(0);
    let mut command = Command::new(shared.frontend);
    command
        .arg("--config-dir")
        .arg(folder.join("config"))
        .arg("launch")
        .arg("--path")
        .arg(shared.cue)
        .args(["--embedded-playtest", "--steps"])
        .arg(((last + 300) * 300_000).to_string())
        .arg("--press")
        .arg(events.join(","))
        .arg("--route-screenshot-dir")
        .arg(&shots)
        .args(["--route-screenshot-interval", "100", "--dump-hash"]);
    if digital {
        command.arg("--digital-pad");
    }
    let card = folder.join("settings.mcd");
    if suite == Suite::Save {
        command.arg("--memcard").arg(&card);
    }
    let words: Vec<String> = std::iter::once(command.get_program())
        .chain(command.get_args())
        .map(|word| word.to_string_lossy().into_owned())
        .collect();
    let (rc, log_text) = crate::replay::run_merged(
        &mut command,
        &folder.join("run.log"),
        Some(Duration::from_secs(600)),
    )?;

    let mut results = Vec::new();
    for (tick, want, detail) in &checks {
        let path = shots.join(format!("tick-{tick:06}.ppm"));
        let got = if path.exists() {
            screen(&path)
        } else {
            ("missing", None)
        };
        let ok = got.0 == *want && (detail.is_none() || got.1 == *detail);
        results.push(json!({
            "tick": tick,
            "expected": [want, detail],
            "actual": [got.0, got.1],
            "passed": ok,
        }));
        if let Some(image) = fs::read(&path).ok().as_deref().and_then(Ppm::parse) {
            save_png(&image, &folder.join(format!("check-{tick:06}.png")))?;
        }
    }
    // Exactly one outer chain-load: an accidental return to the demo
    // carousel and relaunch must not masquerade as a collection return.
    let chainloads = log_text.matches("launcher: chain-loading").count();
    let mut failures: Vec<String> = FAILURE_MARKERS
        .iter()
        .filter(|marker| log_text.contains(**marker))
        .map(|marker| marker.to_string())
        .collect();
    if suite == Suite::Save {
        let data = fs::read(&card).unwrap_or_default();
        let has = |needle: &[u8]| data.windows(needle.len()).any(|window| window == needle);
        if data.len() != 128 * 1024 || !has(b"BESLES-00000CELSTCC1") || !has(b"CCS1\x07\x08") {
            failures.push("changed SFX setting missing from memory card".into());
        }
    }
    let ok = rc == 0
        && chainloads == 1
        && failures.is_empty()
        && results.iter().all(|r| r["passed"] == json!(true));
    let result = json!({
        "name": name,
        "passed": ok,
        "rc": rc,
        "chainloads": chainloads,
        "failures": failures,
        "checks": results,
        "command": words,
    });
    fs::write(folder.join("result.json"), dumps(&result, 2, false) + "\n")?;
    println!("{} {name}", if ok { "PASS" } else { "FAIL" });
    Ok(result)
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(raw, &["frontend", "cue", "out", "jobs", "suite"], &[])?;
    ensure!(
        args.positional.is_empty(),
        "unrecognized arguments: {}",
        args.positional.join(" ")
    );
    let jobs = args.int("jobs")?.unwrap_or(2);
    ensure!(jobs >= 1, "--jobs must be positive");
    let suite_name = args.get("suite").unwrap_or("all");
    ensure!(
        ["basic", "stress", "save", "all"].contains(&suite_name),
        "argument --suite: invalid choice: {suite_name:?} (choose from basic, stress, save, all)"
    );
    let frontend = resolve(&args.require_path("frontend")?)?;
    let cue = resolve(&args.require_path("cue")?)?;
    let out = args.require_path("out")?;
    let image = cue_bin(&cue)?;
    let entries = read_rows(&image)?;
    let (prefix, launch) = menu_presses(&entries, "CELESTE COLLECTION")?;
    ensure!(
        !out.exists(),
        "{}: evidence directory already exists",
        out.display()
    );
    fs::create_dir_all(&out)?;
    let identity = json!({
        "cue": cue.display().to_string(),
        "cue_sha256": sha256_file(&cue)?,
        "bin": image.display().to_string(),
        "bin_sha256": sha256_file(&image)?,
        "frontend": frontend.display().to_string(),
        "frontend_sha256": sha256_file(&frontend)?,
        "experimental_dma_fifo": std::env::var("PSOXIDE_EXPERIMENTAL_DMA_FIFO").ok(),
        "suite": suite_name,
    });
    fs::write(out.join("identity.json"), dumps(&identity, 2, false) + "\n")?;

    let mut cases = Vec::new();
    if matches!(suite_name, "basic" | "all") {
        for cart in [0, 1] {
            for chord in [false, true] {
                for digital in [false, true] {
                    cases.push(Case {
                        cart,
                        chord,
                        digital,
                        suite: Suite::Basic,
                    });
                }
            }
        }
    }
    for (wanted, suite) in [("stress", Suite::Stress), ("save", Suite::Save)] {
        if matches!(suite_name, w if w == wanted || w == "all") {
            for cart in [0, 1] {
                cases.push(Case {
                    cart,
                    chord: false,
                    digital: false,
                    suite,
                });
            }
        }
    }

    let shared = Shared {
        frontend: &frontend,
        cue: &cue,
        out: &out,
        prefix: &prefix,
        launch,
    };
    let next = AtomicUsize::new(0);
    let slots: Mutex<Vec<Option<Result<Value>>>> = Mutex::new(cases.iter().map(|_| None).collect());
    std::thread::scope(|scope| {
        for _ in 0..(jobs as usize).min(cases.len().max(1)) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::SeqCst);
                let Some(case) = cases.get(index) else {
                    break;
                };
                let result = run_case(&shared, *case);
                slots.lock().expect("result slots")[index] = Some(result);
            });
        }
    });
    let mut results = Vec::new();
    for slot in slots.into_inner().expect("result slots") {
        results.push(slot.expect("every case ran")?);
    }
    fs::write(
        out.join("results.json"),
        dumps(&Value::Array(results.clone()), 2, false) + "\n",
    )?;
    let passed = results
        .iter()
        .filter(|r| r["passed"] == json!(true))
        .count();
    println!("{passed}/{} routes passed", results.len());
    Ok(i32::from(passed != results.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(fill: impl Fn(usize, usize) -> [u8; 3]) -> Ppm {
        let mut pixels = Vec::new();
        for y in 0..240 {
            for x in 0..320 {
                pixels.extend(fill(x, y));
            }
        }
        Ppm {
            width: 320,
            height: 240,
            pixels,
        }
    }

    #[test]
    fn menu_frame_names_the_highlighted_cart() {
        let in_probe = |x: usize, y: usize| (156..200).contains(&x) && (202..226).contains(&y);
        // Right cover brighter than the left: cart 1.
        let right = frame(|x, y| {
            if in_probe(x, y) {
                MENU_ACCENT
            } else if (188..268).contains(&x) && (68..148).contains(&y) {
                [200, 200, 200]
            } else {
                [10, 10, 10]
            }
        });
        assert_eq!(classify(&right), ("menu", Some(1)));
        let left = frame(|x, y| {
            if in_probe(x, y) {
                MENU_ACCENT
            } else if (52..132).contains(&x) && (68..148).contains(&y) {
                [200, 200, 200]
            } else {
                [10, 10, 10]
            }
        });
        assert_eq!(classify(&left), ("menu", Some(0)));
    }

    #[test]
    fn credits_pause_and_unknown_frames() {
        assert_eq!(
            classify(&frame(|_, _| CREDITS_BACKGROUND)),
            ("credits", None)
        );
        let pause = frame(|x, y| {
            if (x == 56 || x == 262) && (60..180).contains(&y) {
                PAUSE_BORDER
            } else {
                [0, 0, 0]
            }
        });
        assert_eq!(classify(&pause), ("pause", None));
        assert_eq!(classify(&frame(|_, _| [0, 0, 0])), ("unknown", None));
        // 70 of 120 border rows is not enough for a pause screen.
        let weak = frame(|x, y| {
            if (x == 56 || x == 262) && (60..130).contains(&y) {
                PAUSE_BORDER
            } else {
                [0, 0, 0]
            }
        });
        assert_eq!(classify(&weak), ("unknown", None));
        let small = Ppm {
            width: 2,
            height: 2,
            pixels: vec![0; 12],
        };
        assert_eq!(classify(&small), ("unknown", None));
    }

    #[test]
    fn ppm_reader_requires_an_exact_pixel_payload() {
        let mut data = b"P6\n2 1\n255\n".to_vec();
        data.extend([1, 2, 3, 4, 5, 6]);
        let image = Ppm::parse(&data).unwrap();
        assert_eq!(image.pixel(1, 0), [4, 5, 6]);
        data.push(0);
        assert!(Ppm::parse(&data).is_none());
        assert!(Ppm::parse(b"P5\n1 1\n255\nx").is_none());
    }

    #[test]
    fn basic_route_matches_the_script() {
        let prefix = vec!["400:left:8".to_string(), "1000:cross:12".to_string()];
        let (events, checks) = route(&prefix, 1000, 1, false);
        assert_eq!(
            events,
            [
                "400:left:8",
                "1000:cross:12",
                "2500:right:8",
                "2700:cross:12",
                "3300:cross:12",
                "3900:cross:12",
                "4300:start:8",
                "4460:up:8",
                "4620:cross:8",
                "5000:select:8",
                "5400:cross:8"
            ]
        );
        assert_eq!(
            checks,
            [
                (2400, "menu", Some(0)),
                (4400, "pause", None),
                (4800, "menu", Some(1)),
                (5200, "credits", None),
                (5600, "menu", Some(1)),
            ]
        );
        let (events, checks) = route(&prefix, 1000, 0, true);
        assert!(events.contains(&"4200:select:30".to_string()));
        assert!(events.contains(&"4204:start:26".to_string()));
        assert_eq!(checks.len(), 4);
    }

    #[test]
    fn stress_and_save_routes_end_on_the_menu() {
        let prefix = vec!["1000:cross:12".to_string()];
        let (_, checks) = stress_route(&prefix, 1000, 1);
        assert_eq!(checks.last(), Some(&(17000, "menu", Some(1))));
        let (events, checks) = save_route(&prefix, 1000, 0);
        assert!(events.contains(&"4680:cross:200".to_string()));
        assert_eq!(checks.last(), Some(&(6500, "menu", Some(0))));
    }

    #[test]
    fn bad_suite_and_jobs_are_refused() {
        let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let error = run(&strings(&[
            "--frontend",
            "f",
            "--cue",
            "c",
            "--out",
            "o",
            "--suite",
            "weird",
        ]))
        .unwrap_err();
        assert!(error.0.contains("invalid choice"), "{error}");
        let error = run(&strings(&[
            "--frontend",
            "f",
            "--cue",
            "c",
            "--out",
            "o",
            "--jobs",
            "0",
        ]))
        .unwrap_err();
        assert!(error.0.contains("--jobs must be positive"), "{error}");
    }
}
