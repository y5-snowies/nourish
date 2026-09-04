//! Spawns the subject and drives it.
//!
//! Deliberately a TERMINAL program rather than the GUI panel `../window.stress` uses.
//! That harness is a wayland client testing wayland windows, so a window full of
//! buttons costs nothing; here the thing under test is X11, and a wayland control
//! window would just be one more surface in the way of looking at the X11 ones. A
//! terminal also works over ssh and pipes into a script, which is what a repro wants.
//!
//! Three modes:
//! - `--list`                enumerate the scenarios
//! - `--scenario <name>`     replay one, then hand over to interactive input
//! - `--selftest`            check the command vocabulary with no X server at all
//! - (no arguments)          interactive: type commands, they go to the subject

use std::io::{BufRead, Write};
use std::process::{Child, Command as Proc, Stdio};

use x11_stress::protocol::{all_commands, Command};
use x11_stress::scenario;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--selftest") => std::process::exit(selftest()),
        Some("--list") => {
            for (name, about, steps) in scenario::all() {
                println!("{name:<12} {about} ({} steps)", steps.len());
            }
        }
        Some("--help" | "-h") => usage(),
        Some("--scenario") => match args.get(1) {
            Some(name) => drive(scenario::by_name(name).unwrap_or_else(|| {
                eprintln!("[controller] no scenario {name:?}; --list to see them");
                std::process::exit(2);
            })),
            None => {
                eprintln!("[controller] --scenario needs a name");
                std::process::exit(2);
            }
        },
        None => drive(vec![]),
        Some(other) => {
            eprintln!("[controller] unknown argument {other:?}");
            usage();
            std::process::exit(2);
        }
    }
}

fn usage() {
    eprintln!(
        "x11-stress-controller [--list | --scenario <name> | --selftest]\n\
         \n\
         With no arguments, spawns the subject and forwards each line you type to it.\n\
         Point DISPLAY at the X server under test (y5 logs `XWayland ready on :N`).\n\
         Type `help` at the prompt for the command vocabulary."
    );
}

/// Encode every command and parse it back. Catches a verb that encodes to something
/// the parser does not accept — the failure mode where a scenario line is silently
/// ignored by the subject and the check it was meant to perform never happens.
fn selftest() -> i32 {
    let mut bad = 0;
    for cmd in all_commands() {
        let line = cmd.encode();
        match Command::parse(&line) {
            Some(back) if back == cmd => {}
            Some(back) => {
                eprintln!("[selftest] {line:?} round-tripped to a DIFFERENT command: {back:?}");
                bad += 1;
            }
            None => {
                eprintln!("[selftest] {line:?} does not parse");
                bad += 1;
            }
        }
    }
    // Scenarios are only useful if every step is a command the subject accepts.
    for (name, _, steps) in scenario::all() {
        for step in &steps {
            let line = step.0.encode();
            if Command::parse(&line).as_ref() != Some(&step.0) {
                eprintln!("[selftest] scenario {name}: step {line:?} does not round-trip");
                bad += 1;
            }
        }
    }
    if bad == 0 {
        println!("[selftest] ok — {} commands, {} scenarios", all_commands().len(), scenario::all().len());
    }
    i32::from(bad > 0)
}

/// Find `x11-stress-subject` next to our own executable, the way `window.stress`
/// does — the repo pins one `target-dir` for the whole checkout, so "next to me" is
/// the only thing that stays true wherever it lands.
fn subject_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().unwrap_or_default();
    p.pop();
    p.push("x11-stress-subject");
    p
}

fn spawn() -> Child {
    let path = subject_path();
    Proc::new(&path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| {
            eprintln!("[controller] could not spawn {}: {e}", path.display());
            eprintln!("[controller] build both binaries first: cargo build --release");
            std::process::exit(1);
        })
}

fn drive(steps: Vec<scenario::Step>) {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!(
            "[controller] DISPLAY is unset — the subject is an X11 client and will not \
             connect. y5 logs `XWayland ready on :N` when its server is up."
        );
    }
    let mut child = spawn();
    let mut stdin = child.stdin.take().expect("piped");

    for scenario::Step(cmd, pause) in steps {
        let line = cmd.encode();
        println!("[controller] > {line}");
        if writeln!(stdin, "{line}").is_err() {
            eprintln!("[controller] subject closed its input");
            break;
        }
        let _ = stdin.flush();
        std::thread::sleep(pause);
    }

    println!("[controller] interactive — type commands, `help` for the list, `quit` to finish");
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let line = line.trim();
        if line == "help" {
            help();
            continue;
        }
        if !line.is_empty() && !line.starts_with('#') && Command::parse(line).is_none() {
            println!("[controller] unknown command: {line:?} (type `help`)");
            continue;
        }
        if writeln!(stdin, "{line}").is_err() {
            break;
        }
        let _ = stdin.flush();
        if line == "quit" || line == "exit" {
            break;
        }
    }
    let _ = child.wait();
}

fn help() {
    println!("commands (ids are 1, 2, 3 … in creation order — `info` lists them):");
    for cmd in all_commands() {
        println!("  {}", cmd.encode());
    }
    println!("\nscenarios (--scenario <name>):");
    for (name, about, _) in scenario::all() {
        println!("  {name:<12} {about}");
    }
}
