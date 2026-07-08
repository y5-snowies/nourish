//! Interactive y5 installer.
//!
//! Run from the unzipped artifact (the prebuilt binaries + templates sit next to
//! this executable). It:
//!   1. detects the GPU and installs the chosen dnf package/driver groups,
//!   2. prompts the default Y5 Desktop configuration (values propagate to every preset),
//!   3. lays down all session presets (renderer × experimental × sync, + Custom),
//!   4. optionally installs the dev tool window, MX daemon, polkit agent and PAM lock.
//!
//! Re-runnable: every file is overwritten. `--dry-run` prints the plan without
//! touching the system. The steps live in the sibling `execute.*` crates.

use compositor_installer_process_layout_compute_base as layout;
use compositor_installer_process_main_execute_configure as configure;
use compositor_installer_process_main_execute_info as info;
use compositor_installer_process_main_execute_packages as packages;
use compositor_installer_process_main_execute_plan as plan;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dry_run = args.iter().any(|a| a == "--dry-run" || a == "-n");
    if args.iter().any(|a| a == "-h" || a == "--help") {
        info::print_help();
        return;
    }
    if args.iter().any(|a| a == "--emit-presets") {
        info::emit_presets();
        return;
    }
    // Pre-CI package-name verifier hook: dump a manager's package names and exit.
    if let Some(spec) = args.iter().find_map(|a| a.strip_prefix("--emit-packages=")) {
        std::process::exit(if info::emit_packages(spec) { 0 } else { 2 });
    }
    // NixOS module generator (used by prepare.sh to pre-stage nixos/configuration-y5.nix).
    if args.iter().any(|a| a == "--emit-nixos") {
        info::emit_nixos();
        return;
    }
    // NixOS flake generator (nixosModules.default — for flakes-based configs).
    if args.iter().any(|a| a == "--emit-nixos-flake") {
        info::emit_nixos_flake();
        return;
    }

    // Must run as the normal user, NOT root/sudo: per-action sudo handles the steps that
    // need privilege, and $HOME must be the user's so settings.json + the user systemd
    // units land in their ~/.config rather than /root.
    if layout::is_root() {
        eprintln!("y5-install: do not run as root or with sudo.");
        eprintln!("Run it as your normal user — it invokes sudo itself only for the steps that");
        eprintln!("need root, so your configuration lands in $HOME/.config, not /root.");
        std::process::exit(1);
    }

    println!("=== y5 compositor installer ===");
    if dry_run {
        println!("(dry-run: no changes will be made)\n");
    }

    let stage = layout::Stage::resolve();
    println!("Artifact staging dir: {}", stage.root.display());

    // 1) Packages / drivers.
    // A real install failure aborts here rather than pressing on and leaving a
    // half-installed system.
    if let Err(e) = packages::select_and_install(dry_run) {
        eprintln!("\nInstallation aborted: {e}");
        std::process::exit(1);
    }

    // 2-3) Default Y5 Desktop configuration + presets (incl. optional Custom).
    let presets = configure::gather();

    // 4-5) Optional components, plan assembly, apply.
    plan::build_and_apply(&stage, &presets, dry_run);
}
