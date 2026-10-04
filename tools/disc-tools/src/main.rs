//! Host tools for the PSoXide demo disc. One binary with a subcommand for each
//! job the disc's scripts do: verify and receipt a pressing, replay the
//! release checks through the emulator, cook the menu assets, stage a deploy.
//!
//! `disc-tools help` lists the subcommands. Each one reads its own options;
//! see the module that owns it.

#[macro_use]
mod util;
mod args;
mod disc;

mod audio;
mod banner;
mod beatgrid;
mod celeste;
mod chainloads;
mod components;
mod deploy;
mod icons;
mod label;
mod lineup;
mod programs;
mod quake;
mod receipt;
mod shots;
mod web;

use util::Result;

type Command = fn(&[String]) -> Result<i32>;

/// Subcommand, the module that owns it, and one line of help.
const COMMANDS: &[(&str, Command, &str)] = &[
    ("components", components::run, "verify the pinned component tuple and bootstrap it"),
    ("release-receipt", receipt::run, "create or verify a combined-pressing receipt"),
    ("lineup", lineup::run, "prepare and receipt a pressing from a lineup file"),
    ("quake", quake::run, "verify, receipt or repin the Quake payload; headless chain-load check"),
    ("chainloads", chainloads::run, "replay release-critical chain-loads twice, byte-identical"),
    ("programs", programs::run, "boot the independent games and the Arcade guests headless"),
    ("celeste-nav", celeste::run, "check Celeste return and credits navigation through the pressing"),
    ("audio-relocation", audio::run, "compare every pressed audio sector with its source cue"),
    ("deploy-public", deploy::run, "stage (and optionally publish) the public itch.io packages"),
    ("web-delivery", web::run, "split a pressed disc into the browser emulator's delivery set"),
    ("cook-shot", shots::run, "cook a screenshot into the menu panel format"),
    ("banner", banner::run, "cook the launcher banner texture"),
    ("icons", icons::run, "cook the credits link icons"),
    ("beatgrid", beatgrid::run, "find the beat grid of a raw CD-DA track"),
    ("disc-label", label::run, "print the disc label SVG"),
];

fn usage() {
    eprintln!("usage: disc-tools <command> [options]\n\ncommands:");
    for (name, _, help) in COMMANDS {
        eprintln!("  {name:<17} {help}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = args.first() else {
        usage();
        std::process::exit(2);
    };
    if matches!(name.as_str(), "help" | "--help" | "-h") {
        usage();
        return;
    }
    let Some((_, run, _)) = COMMANDS.iter().find(|(n, _, _)| n == name) else {
        eprintln!("disc-tools: unknown command {name:?}");
        usage();
        std::process::exit(2);
    };
    match run(&args[1..]) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{name}: {error}");
            std::process::exit(2);
        }
    }
}
