use std::path::{Path, PathBuf};
use compositor_installer_process_layout_compute_stage::{Action, Source, is_root};

/// Execute (or, for `dry_run`, print) the plan. Returns Err on the first failure.
pub fn apply(actions: &[Action], dry_run: bool) -> Result<(), String> {
    let am_root = is_root();
    for act in actions {
        match act {
            Action::Place { dest, source, mode, root } => {
                place(dest, source, *mode, *root, am_root, dry_run)?;
            }
            Action::SystemctlUser(args) => {
                run("systemctl", &prepend("--user", args), false, am_root, dry_run)?;
            }
            Action::UdevReload => {
                run("udevadm", &svec(&["control", "--reload"]), true, am_root, dry_run)?;
                run("udevadm", &svec(&["trigger"]), true, am_root, dry_run)?;
            }
        }
    }
    Ok(())
}


fn place(dest: &Path, source: &Source, mode: u32, root: bool, am_root: bool, dry_run: bool) -> Result<(), String> {
    // Materialize the source as a concrete path (writing text to a temp file).
    let src_path: PathBuf = match source {
        Source::Copy(p) => p.clone(),
        Source::Text(t) => {
            let tmp = std::env::temp_dir().join(format!(
                "y5-install-{}",
                dest.file_name().and_then(|s| s.to_str()).unwrap_or("file")
            ));
            if dry_run {
                println!("  [dry-run] write {} ({} bytes) -> staged temp", dest.display(), t.len());
            } else {
                std::fs::write(&tmp, t).map_err(|e| format!("write temp {}: {e}", tmp.display()))?;
            }
            tmp
        }
    };

    // Stage the file next to its final destination, then atomically rename it into
    // place. Two reasons this beats writing `dest` directly:
    //   * Atomic: a crash mid-copy never leaves a half-written/truncated binary.
    //   * No ETXTBSY: writing over a *running* executable (the compositor, the polkit
    //     agent, the mx daemon — any of which may be live during a re-install) fails
    //     with "Text file busy". rename(2) just swaps the directory entry; the running
    //     process keeps the old inode while new launches pick up the new one.
    // The temp lives in the same directory as `dest` so the rename stays on one
    // filesystem (cross-FS rename would fall back to a non-atomic copy).
    let mode_s = format!("{:o}", mode);
    let file = dest.file_name().and_then(|s| s.to_str()).unwrap_or("file");
    let tmp_dest = dest.with_file_name(format!(".{file}.y5-install-tmp"));
    let tmp_s = tmp_dest.to_string_lossy().to_string();

    // `install -D` creates parent dirs and sets the mode on the staged temp file.
    let stage_args = svec(&["-D", "-m", &mode_s, &src_path.to_string_lossy(), &tmp_s]);
    run("install", &stage_args, root, am_root, dry_run)?;

    // `mv -f` is rename(2): atomic, and immune to ETXTBSY on a running target.
    run("mv", &svec(&["-f", &tmp_s, &dest.to_string_lossy()]), root, am_root, dry_run)
}

/// Run a command, prefixing `sudo` when the action needs root and we are not root.
fn run(cmd: &str, args: &[String], needs_root: bool, am_root: bool, dry_run: bool) -> Result<(), String> {
    let use_sudo = needs_root && !am_root;
    if dry_run {
        let prefix = if use_sudo { "sudo " } else { "" };
        println!("  [dry-run] {prefix}{cmd} {}", args.join(" "));
        return Ok(());
    }
    let (program, full_args): (&str, Vec<String>) = if use_sudo {
        ("sudo", prepend(cmd, args))
    } else {
        (cmd, args.to_vec())
    };
    let status = std::process::Command::new(program)
        .args(&full_args)
        .status()
        .map_err(|e| format!("run {cmd}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} exited with {status}"))
    }
}

fn svec(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

fn prepend(first: &str, rest: &[String]) -> Vec<String> {
    let mut v = vec![first.to_string()];
    v.extend_from_slice(rest);
    v
}
