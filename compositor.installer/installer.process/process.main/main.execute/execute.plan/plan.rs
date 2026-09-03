use compositor_installer_process_config_parse_base as cfg;
use compositor_installer_process_config_parse_base::prompt;
use compositor_installer_process_layout_compute_base as layout;

/// List the presets, prompt the optional components, build the plan, and apply
/// it (or print it for `dry_run`). Exits the process with code 1 on failure.
pub fn build_and_apply(stage: &layout::Stage, presets: &[cfg::Preset], dry_run: bool) {
    println!("\nPresets to install:");
    for p in presets {
        println!("  - {:<22} desktop={:<20} binary={}", p.label, p.desktop_name, p.binary);
    }

    let want_devtool = prompt::yes_no("devtool", "Install the developer tool window", true);
    let want_pam = prompt::yes_no("pam-lock", "Install the PAM lock policy (/etc/pam.d/y5-lock)", true);
    let want_mx = prompt::yes_no("mx-daemon", "Install the MX Master gesture daemon", false);
    let want_polkit = prompt::yes_no("polkit", "Install the polkit authentication agent (systemd service)", true);

    let mut plan: Vec<layout::Action> = Vec::new();
    plan.extend(layout::binary_actions(stage, presets));
    // Seed settings.json from the prompted config (the wrappers no longer write it) — but
    // ONLY when the user has none. It is the single file in the plan that belongs to them,
    // and every other action here is an unconditional overwrite. See
    // `layout::settings_exists`.
    if let Some(p) = presets.first() {
        match layout::settings_exists() {
            false => plan.push(layout::settings_action(p)),
            true => println!(
                "\nsettings.json already exists — keeping it (edit with 'y5.compositor.settings')\n  {}",
                layout::settings_path().display()
            ),
        }
    }
    // `y5.compositor.update` + `y5.compositor.uninstall`, keyed to the first preset's
    // binary (the marker) and names (what uninstall removes).
    if let Some(p) = presets.first() {
        plan.extend(layout::maintenance_actions(p));
    }
    for p in presets {
        plan.extend(layout::preset_actions(p));
    }
    if want_devtool {
        plan.extend(layout::devtool_actions(stage));
    }
    if want_pam {
        plan.extend(layout::pam_actions(stage));
    }
    if want_mx {
        plan.extend(layout::mx_actions(stage));
    }
    if want_polkit {
        plan.extend(layout::polkit_actions(stage));
    }
    // X11 support is no longer a component to opt into — the compositor runs Xwayland
    // itself — so there is nothing to prompt for. What remains is retiring the
    // satellite a previous install left running; that is unconditional, and emits
    // nothing on a machine that never had it.
    plan.extend(layout::xwayland_retire_actions());
    // Re-scan units once everything is placed.
    plan.push(layout::Action::SystemctlUser(vec!["daemon-reload".into()]));

    println!("\n-- Applying {} actions --", plan.len());
    match layout::apply(&plan, dry_run) {
        Ok(()) => {
            println!("\nDone.{}", if dry_run { " (dry-run)" } else { "" });
            if !dry_run {
                println!(
                    "\nThe \"Y5 Compositor\" session is installed — log out and pick it in your \
                     display manager.\n\n  \
                     • Run this installer again (or just `y5.compositor.settings`) anytime to \
                     reconfigure or reinstall.\n  \
                     • To update, visit https://nourish.snowies.com to fetch the latest installer."
                );
            }
        }
        Err(e) => {
            eprintln!("\nInstall failed: {e}");
            std::process::exit(1);
        }
    }
}
