//! `disc-tools journeys`: the headless psoxide-gate journeys of a pressed disc, generated from its
//! `members.json` (see `members`) and its lineup file, so a new member needs no hand-written
//! journey.
//!
//! For the pressed image (`--cue`) and the launcher's link map (`--map`) it writes, into `--out`:
//!
//! * `chain-NN-<slug>.toml`, one per member: move the shelf to the member, launch it, see the
//!   launcher hand off, then the member's own text resident (its four signature words), its own
//!   VBlank counter cleared by its start and counting past 120, and again after 600 ticks;
//! * `carousel.toml`: the shelf moves both ways, the credits open and close, the attract tour
//!   starts and any button ends it;
//! * `pad-refusal.toml`: a digital pad gets the launcher's notice and stays there;
//! * `covers-all.toml`: every shelf entry focused in turn, a frame each.
//!
//! A lineup row may carry a `journey` object with hints the generator cannot read from the image:
//! `{"timeout": N}` (ticks to wait for the member's start; default 6000), `{"movie": "in_game"}`
//! (a film plays once the member runs: 14 frames) or `{"movie": "intro"}` (the same, but played
//! by an intro executable that hands over to the member's own, so the member's text is looked for
//! after the frames), `{"opening": N}` (Start Game and the first profile, then N frames of the
//! opening cinematics, every 240 ticks). The restored games' `title_probe` comes through
//! `members.json`.

use std::fs;
use std::path::Path;

use regex::Regex;
use serde_json::Value;

use crate::args::Args;
use crate::util::{read_json, Error, Result};

/// Ticks to wait for a member's start unless its lineup row says otherwise.
const DEFAULT_TIMEOUT: u64 = 6000;
/// Ticks a chain journey may run beyond the wait for the member's start.
const TAIL_TICKS: u64 = 4000;
/// `LAUNCHER_STATE` holds the value for an unselected launch: all ones.
const NO_LAUNCH: u64 = 4_294_967_295;

/// Where a film plays, relative to the start of the member's own executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Movie {
    None,
    InGame,
    Intro,
}

/// What a lineup row says about how its member's journey differs from the plain one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hints {
    timeout: u64,
    movie: Movie,
    opening: u64,
}

impl Hints {
    fn of(row: Option<&Value>) -> Result<Hints> {
        let journey = row.map(|r| &r["journey"]).unwrap_or(&Value::Null);
        let movie = match journey["movie"].as_str() {
            None => Movie::None,
            Some("in_game") => Movie::InGame,
            Some("intro") => Movie::Intro,
            Some(other) => bail!("journey.movie {other:?} is not in_game or intro"),
        };
        Ok(Hints {
            timeout: journey["timeout"].as_u64().unwrap_or(DEFAULT_TIMEOUT),
            movie,
            opening: journey["opening"].as_u64().unwrap_or(0),
        })
    }
}

/// A member of the shelf as the journeys need it.
struct Member {
    index: usize,
    id: String,
    words: Vec<(String, String)>,
    vblank: String,
    probe: Option<Probe>,
    hints: Hints,
}

struct Probe {
    sym: String,
    addr: String,
    op: String,
    value: u64,
}

impl Member {
    fn slug(&self) -> String {
        self.id.to_lowercase().replace(' ', "-")
    }

    /// `u32(a) == w && ...`: the member's text is resident.
    fn resident(&self) -> String {
        let terms: Vec<String> = self
            .words
            .iter()
            .map(|(a, w)| format!("u32({a}) == {w}"))
            .collect();
        terms.join(" && ")
    }
}

fn members_of(members: &Value, lineup: &Value) -> Result<Vec<Member>> {
    let rows = lineup["programs"].as_array();
    let list = members
        .as_array()
        .ok_or_else(|| Error("members.json is not a list".into()))?;
    let text = |value: &Value, what: &str| -> Result<String> {
        value
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| Error(format!("members.json: {what} is not text")))
    };
    let mut out = Vec::new();
    for (position, member) in list.iter().enumerate() {
        let id = text(&member["id"], "id")?;
        let index = member["index"]
            .as_u64()
            .ok_or_else(|| Error(format!("members.json: {id}: index")))?
            as usize;
        ensure!(
            index == position,
            "members.json: {id}: index {index} is not its position"
        );
        let mut words = Vec::new();
        for pair in member["words"].as_array().into_iter().flatten() {
            words.push((text(&pair[0], "word address")?, text(&pair[1], "word")?));
        }
        ensure!(!words.is_empty(), "members.json: {id}: no signature words");
        let probe = match member["title_probe"].as_object() {
            None => None,
            Some(p) => Some(Probe {
                sym: text(&p["sym"], "probe sym")?,
                addr: text(&p["addr"], "probe addr")?,
                op: text(&p["op"], "probe op")?,
                value: p["value"].as_u64().unwrap_or(0),
            }),
        };
        let row = rows.and_then(|r| r.iter().find(|r| r["name"].as_str() == Some(&id)));
        out.push(Member {
            index,
            vblank: text(&member["vblank"], "vblank")?,
            words,
            probe,
            hints: Hints::of(row)?,
            id,
        });
    }
    ensure!(!out.is_empty(), "members.json lists no members");
    Ok(out)
}

/// A wait or hold carries its timeout on the step, not inside the condition table.
fn move_timeouts(text: &str) -> String {
    let pattern = Regex::new(r"(?m)^((?:wait_until|hold_until) = \{.*?), timeout = (\d+) \}$")
        .expect("static pattern");
    pattern.replace_all(text, "$1 }\ntimeout = $2").into_owned()
}

fn head(name: &str, title: &str, cue: &str, map: &str, pad: &str, max_ticks: u64) -> String {
    format!(
        "name = \"{name}\"\ntitle = \"{title}\"\ndisc = [\"{cue}\"]\nsymbols = [\"{map}\"]\n\
         pad = \"{pad}\"\nmax_ticks = {max_ticks}\n\n"
    )
}

/// A `LAUNCHER_STATE` condition: word at byte `offset`.
fn ls(offset: u32, op: &str, value: u64) -> String {
    format!(
        "{{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = {offset}, op = \"{op}\", \
         value = {value} }}"
    )
}

/// An expression condition with its own timeout, in the single-line form `move_timeouts` splits.
fn expr_wait(expr: &str, timeout: u64) -> String {
    format!("wait_until = {{ kind = \"expr\", expr = \"{expr}\", timeout = {timeout} }}\n\n")
}

fn shelf(count: usize) -> String {
    format!(
        "[[step]]  # the shelf takes input: the DualShock that powered up digital was switched to analog\n\
         wait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 2, timeout = 1500 }}\n\n\
         [[step]]\ncheckpoint = \"shelf\"\ngolden = false\nassert = [{}, {}, {}]\n\n",
        ls(8, "eq", count as u64),
        ls(12, "eq", 2),
        ls(4, "eq", 0)
    )
}

/// One press of a direction and the focus it must reach.
fn shelf_step(button: &str, focus: usize) -> String {
    format!(
        "[[step]]\npress = [\"{button}\"]\nhold = 4\nwait = 8\n\n[[step]]\nwait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 4, op = \"eq\", value = {focus}, timeout = 30 }}\n\n"
    )
}

fn checkpoint(name: &str) -> String {
    format!("[[step]]\ncheckpoint = \"{name}\"\ngolden = false\n\n")
}

fn chain(m: &Member, count: usize, cue: &str, map: &str) -> String {
    let (i, id) = (m.index, &m.id);
    let words = m.resident();
    let vblank = &m.vblank;
    let wait = m.hints.timeout;
    let running = format!("{words} && u32({vblank}) >= 120");
    let mut t = head(
        &format!("chain-{}", m.slug()),
        &format!("{id} through the launcher"),
        cue,
        map,
        "dualshock",
        wait + TAIL_TICKS,
    );
    t += &shelf(count);
    if i > 0 {
        t += &format!(
            "[[step]]  # move the shelf one case at a time until {id} is focused\nuntil = {}\nmax_iter = {count}\ntimeout = 1200\n[[step.body]]\npress = [\"right\"]\nhold = 4\nwait = 8\n\n",
            ls(4, "eq", i as u64)
        );
    }
    t += &format!(
        "[[step]]\ncheckpoint = \"focused\"\ngolden = false\nassert = [{}, {}]\n\n\
         [[step]]\npress = [\"cross\"]\nhold = 4\n\n\
         [[step]]  # the launch started for this member\n\
         wait_until = {{ kind = \"expr\", expr = \"u32(LAUNCHER_STATE) >= 5 && u32(LAUNCHER_STATE[4]) == {i}\", timeout = 60 }}\n\n\
         [[step]]  # hand-off to the loader\n\
         wait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 6, timeout = 200 }}\n\n",
        ls(4, "eq", i as u64),
        ls(0, "eq", 2)
    );
    // The loader copies text and data but not .bss, so until the member's start clears it the
    // counter's address holds whatever the launcher left there: first see it fresh (under 60),
    // then count to 120. A member whose game executable comes after an intro (Half-Life) is
    // checked after the intro's frames; the others are checked before their frames.
    let fresh = format!(
        "[[step]]  # {id} started: its text resident and its counter cleared by its own start\n{}",
        expr_wait(&format!("{words} && u32({vblank}) < 60"), wait)
    );
    let intro_first = m.hints.movie == Movie::Intro;
    if !intro_first {
        t += &fresh;
    }
    if m.hints.movie != Movie::None {
        for k in 0..14 {
            let gap = if k == 0 { 120 } else { 60 };
            t += &format!("[[step]]\nwait = {gap}\n\n");
            t += &checkpoint(&format!("movie-{:02}", k + 1));
        }
    }
    if intro_first {
        t += &fresh;
    }
    t += &format!(
        "[[step]]  # {id} runs: its text resident and its own VBlank counter past 120\n{}\
         [[step]]\ncheckpoint = \"running\"\ngolden = false\nassert = [{{ kind = \"expr\", expr = \"{running}\" }}]\n\n\
         [[step]]\nwait = 600\n\n[[step]]\ncheckpoint = \"later\"\ngolden = false\n\
         assert = [{{ kind = \"expr\", expr = \"{words} && u32({vblank}) >= 240\" }}]\n",
        expr_wait(&running, wait)
    );
    if let Some(p) = &m.probe {
        let op = if p.op == "ge" { ">=" } else { "==" };
        let cond = format!("{words} && u32({}) {op} {}", p.addr, p.value);
        if p.op == "ge" {
            t += &format!(
                "\n[[step]]  # {} seen below its target first (cleared, not left over)\n{}",
                p.sym,
                expr_wait(&format!("{words} && u32({}) < {}", p.addr, p.value), 4000)
                    .trim_end_matches('\n')
            );
            t.push('\n');
        }
        t += &format!(
            "\n[[step]]  # the title state the game publishes ({})\n{}\
             [[step]]\ncheckpoint = \"title\"\ngolden = false\nassert = [{{ kind = \"expr\", expr = \"{cond}\" }}]\n",
            p.sym,
            expr_wait(&cond, 4000)
        );
    }
    if m.hints.opening > 0 {
        // The opening cinematics play on a new game: Start Game, then the first (empty) profile
        // on the fresh card. Open loop: the shipping disc publishes no menu state.
        t += "\n[[step]]\npress = [\"cross\"]\nhold = 4\nwait = 90\n\n[[step]]\ncheckpoint = \"profiles\"\ngolden = false\n\n";
        t += "[[step]]\npress = [\"cross\"]\nhold = 4\n\n";
        for k in 0..m.hints.opening {
            t += &format!("[[step]]\nwait = 240\n\n");
            t += &checkpoint(&format!("opening-{:02}", k + 1));
        }
    }
    move_timeouts(&t)
}

fn carousel(count: usize, cue: &str, map: &str) -> String {
    let mut t = head("carousel", "Launcher carousel", cue, map, "dualshock", 4000);
    t += &shelf(count);
    for k in 1..4 {
        t += &shelf_step("right", k);
    }
    t += &checkpoint("right-3");
    for k in [2, 1, 0] {
        t += &shelf_step("left", k);
    }
    t += &format!(
        "[[step]]\npress = [\"left\"]\nhold = 4\nwait = 8\n\n[[step]]\ncheckpoint = \"left-edge\"\ngolden = false\n\
         assert = [{}, {}]\n\n\
         [[step]]\npress = [\"select\"]\nhold = 4\nwait = 8\n\n\
         [[step]]\nwait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 4, timeout = 30 }}\n\n\
         [[step]]\ncheckpoint = \"credits\"\ngolden = false\n\n\
         [[step]]\npress = [\"circle\"]\nhold = 4\nwait = 8\n\n\
         [[step]]\nwait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 2, timeout = 30 }}\n\n\
         [[step]]  # twenty idle seconds start the attract tour\n\
         wait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 3, timeout = 1500 }}\n\n\
         [[step]]\ncheckpoint = \"attract\"\ngolden = false\n\n\
         [[step]]\npress = [\"cross\"]\nhold = 4\nwait = 8\n\n\
         [[step]]  # any button ends the tour, and is not also a launch\n\
         wait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 2, timeout = 30 }}\n\n\
         [[step]]\nwait = 120\n\n\
         [[step]]\ncheckpoint = \"back-on-shelf\"\ngolden = false\nassert = [{}, {}]\n",
        ls(4, "eq", 0),
        ls(0, "eq", 2),
        ls(0, "eq", 2),
        ls(16, "eq", NO_LAUNCH)
    );
    move_timeouts(&t)
}

/// A pure digital pad gets the launcher's notice and stays there.
fn pad_refusal(cue: &str, map: &str) -> String {
    let mut t = head(
        "pad-refusal",
        "Digital pad refused",
        cue,
        map,
        "digital",
        2000,
    );
    t += &format!(
        "[[step]]\nwait_until = {{ kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 1, timeout = 1500 }}\n\n\
         [[step]]\npress = [\"cross\"]\nhold = 4\nwait = 300\n\n\
         [[step]]\ncheckpoint = \"notice\"\ngolden = false\nassert = [{}, {}, {}]\n",
        ls(0, "eq", 1),
        ls(12, "eq", 1),
        ls(16, "eq", NO_LAUNCH)
    );
    move_timeouts(&t)
}

/// Every shelf entry focused in turn, a frame each (the review frames of the launcher's covers).
fn covers_all(count: usize, cue: &str, map: &str) -> String {
    let max_ticks = 4000 + 1000 * count as u64;
    let mut t = head(
        "covers-all",
        "Launcher covers: every member on the shelf",
        cue,
        map,
        "dualshock",
        max_ticks,
    );
    t += "[[step]]\nwait_until = { kind = \"ram\", sym = \"LAUNCHER_STATE\", offset = 0, op = \"eq\", value = 2 }\ntimeout = 1500\n\n[[step]]\nwait = 150\n\n";
    t += &format!(
        "[[step]]\ncheckpoint = \"shelf-00\"\ngolden = false\nassert = [{}, {}]\n\n",
        ls(4, "eq", 0),
        ls(8, "eq", count as u64)
    );
    for k in 1..count {
        t += &format!(
            "[[step]]\nuntil = {}\nmax_iter = 3\ntimeout = 300\n[[step.body]]\npress = [\"right\"]\nhold = 4\nwait = 8\n\n\
             [[step]]\nwait = 150\n\n\
             [[step]]\ncheckpoint = \"shelf-{k:02}\"\ngolden = false\nassert = [{}, {}]\n\n",
            ls(4, "eq", k as u64),
            ls(4, "eq", k as u64),
            ls(0, "eq", 2)
        );
    }
    t.truncate(t.trim_end_matches('\n').len() + 1);
    t
}

/// Every journey of the disc as `(file name, text)`.
fn generate(members: &[Member], cue: &str, map: &str) -> Vec<(String, String)> {
    let count = members.len();
    let mut files: Vec<(String, String)> = members
        .iter()
        .map(|m| {
            (
                format!("chain-{:02}-{}.toml", m.index, m.slug()),
                chain(m, count, cue, map),
            )
        })
        .collect();
    files.push(("carousel.toml".into(), carousel(count, cue, map)));
    files.push(("pad-refusal.toml".into(), pad_refusal(cue, map)));
    files.push(("covers-all.toml".into(), covers_all(count, cue, map)));
    files
}

pub fn run(raw: &[String]) -> Result<i32> {
    let args = Args::parse(raw, &["members", "lineup", "cue", "map", "out"], &[])?;
    let members = members_of(
        &read_json(&args.require_path("members")?)?,
        &read_json(&args.require_path("lineup")?)?,
    )?;
    let out = args.require_path("out")?;
    let (cue, map) = (args.require("cue")?, args.require("map")?);
    fs::create_dir_all(&out)?;
    let files = generate(&members, cue, map);
    for (name, text) in &files {
        fs::write(Path::new(&out).join(name), text)?;
    }
    println!("{} journeys: {}", files.len(), out.display());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CUE: &str = "/d/disc.cue";
    const MAP: &str = "/d/launcher.map";

    fn member(index: usize, id: &str) -> Value {
        json!({
            "index": index, "id": id, "vblank": "0x80001000",
            "words": [["0x80010100", "0x1"], ["0x80010200", "0x2"]],
            "title_probe": null,
        })
    }

    fn built(rows: Value, lineup: Value) -> Vec<(String, String)> {
        let members = members_of(&rows, &lineup).unwrap();
        generate(&members, CUE, MAP)
    }

    fn file<'a>(files: &'a [(String, String)], name: &str) -> &'a str {
        &files.iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn one_chain_journey_per_member_plus_the_shelf_journeys() {
        let files = built(
            json!([member(0, "FIRST ONE"), member(1, "SECOND")]),
            json!({"programs": []}),
        );
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "chain-00-first-one.toml",
                "chain-01-second.toml",
                "carousel.toml",
                "pad-refusal.toml",
                "covers-all.toml"
            ]
        );
    }

    #[test]
    fn a_new_member_changes_no_other_chain_journey_but_the_shelf_count() {
        let two = built(
            json!([member(0, "A"), member(1, "B")]),
            json!({"programs": []}),
        );
        let three = built(
            json!([member(0, "A"), member(1, "B"), member(2, "C")]),
            json!({"programs": []}),
        );
        let (a2, a3) = (
            file(&two, "chain-00-a.toml"),
            file(&three, "chain-00-a.toml"),
        );
        assert_eq!(
            a2.replace("value = 2 }, {", "value = 3 }, {"),
            a3.replace("value = 2 }, {", "value = 3 }, {")
        );
        assert!(a3.contains("offset = 8, op = \"eq\", value = 3"));
        assert!(file(&three, "chain-02-c.toml").contains("max_iter = 3"));
        assert!(file(&three, "covers-all.toml").contains("checkpoint = \"shelf-02\""));
    }

    #[test]
    fn the_first_member_needs_no_shelf_moves_and_a_later_one_presses_right() {
        let files = built(
            json!([member(0, "A"), member(1, "B")]),
            json!({"programs": []}),
        );
        assert!(!file(&files, "chain-00-a.toml").contains("[[step.body]]"));
        assert!(file(&files, "chain-01-b.toml").contains("until = { kind = \"ram\""));
        assert!(file(&files, "chain-01-b.toml").contains("[[step.body]]\npress = [\"right\"]"));
    }

    #[test]
    fn waits_carry_their_timeout_on_the_step() {
        let files = built(json!([member(0, "A")]), json!({"programs": []}));
        for (_, text) in &files {
            assert!(!text.contains(", timeout = 6000 }"), "{text}");
        }
        let chain = file(&files, "chain-00-a.toml");
        assert!(chain.contains("u32(0x80001000) < 60\" }\ntimeout = 6000\n"));
        assert!(chain.contains("max_ticks = 10000"));
    }

    #[test]
    fn lineup_hints_set_the_wait_the_films_and_the_opening() {
        let lineup = json!({"programs": [
            {"name": "SLOW", "journey": {"timeout": 9000}},
            {"name": "FILM", "journey": {"movie": "in_game"}},
            {"name": "INTRO", "journey": {"movie": "intro"}},
            {"name": "OPEN", "journey": {"opening": 3}},
        ]});
        let files = built(
            json!([
                member(0, "SLOW"),
                member(1, "FILM"),
                member(2, "INTRO"),
                member(3, "OPEN")
            ]),
            lineup,
        );
        assert!(file(&files, "chain-00-slow.toml").contains("max_ticks = 13000"));
        let film = file(&files, "chain-01-film.toml");
        assert!(film.contains("checkpoint = \"movie-14\""));
        assert!(!film.contains("movie-15"));
        // A film inside the game follows the member's start; an intro's film precedes it.
        let started = |text: &str| text.find("started: its text resident").unwrap();
        assert!(started(film) < film.find("movie-01").unwrap());
        let intro = file(&files, "chain-02-intro.toml");
        assert!(started(intro) > intro.find("movie-14").unwrap());
        let open = file(&files, "chain-03-open.toml");
        assert!(open.contains("opening-03") && !open.contains("opening-04"));
        assert!(!file(&files, "chain-00-slow.toml").contains("opening-01"));
    }

    #[test]
    fn a_title_probe_waits_below_the_target_first_for_ge_only() {
        let mut ge = member(0, "G");
        ge["title_probe"] =
            json!({"sym": "gamecycle", "addr": "0x80002000", "op": "ge", "value": 100});
        let mut eq = member(1, "E");
        eq["title_probe"] = json!({"sym": "cursor", "addr": "0x80003000", "op": "eq", "value": 4});
        let files = built(json!([ge, eq]), json!({"programs": []}));
        let ge = file(&files, "chain-00-g.toml");
        assert!(ge.contains("gamecycle seen below its target first"));
        assert!(ge.contains("u32(0x80002000) >= 100"));
        let eq = file(&files, "chain-01-e.toml");
        assert!(!eq.contains("seen below"));
        assert!(eq.contains("u32(0x80003000) == 4"));
    }

    #[test]
    fn members_out_of_order_and_bad_hints_are_refused() {
        let error = members_of(&json!([member(1, "A")]), &json!({}))
            .err()
            .unwrap();
        assert!(error.0.contains("not its position"), "{error}");
        let lineup = json!({"programs": [{"name": "A", "journey": {"movie": "sometimes"}}]});
        let error = members_of(&json!([member(0, "A")]), &lineup).err().unwrap();
        assert!(error.0.contains("journey.movie"), "{error}");
    }

    #[test]
    fn the_private_lineup_carries_the_hints_the_slow_and_film_members_need() {
        let root = crate::util::repo_root().unwrap();
        let lineup = read_json(&root.join("release/lineup-v0.59-private.json")).unwrap();
        let hints = |name: &str| {
            let row = lineup["programs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["name"] == name);
            Hints::of(row).unwrap()
        };
        assert_eq!(
            hints("HALF-LIFE"),
            Hints {
                timeout: 12000,
                movie: Movie::Intro,
                opening: 0
            }
        );
        assert_eq!(
            hints("HOLLOW KNIGHT"),
            Hints {
                timeout: 17000,
                movie: Movie::InGame,
                opening: 32
            }
        );
        assert_eq!(hints("360X").movie, Movie::InGame);
        assert_eq!(hints("NITROXIDE").timeout, DEFAULT_TIMEOUT);
    }
}
