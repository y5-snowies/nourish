//! `portal-stress` — does this session actually have a working desktop portal?
//!
//! Point it at any bus. Run it on your ordinary host session to establish what
//! "working" looks like, then inside a nested compositor to see what is missing.
//!
//! Exit status is 0 unless a required check failed, so it can gate a script.

mod bus;
mod interactive;
mod portal;
mod report;
mod secrets;
mod services;

use report::{Report, Status};
use zbus::blocking::Connection;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("usage: portal-stress [--interactive]");
        println!();
        println!("  (default)      read-only: names, versions, config, environment");
        println!("  --interactive  additionally open a file chooser, take a screenshot,");
        println!("                 and start a screencast — opens real windows");
        return;
    }
    let run_interactive = args.iter().any(|a| a == "--interactive");

    let mut report = Report::default();
    let connection = match check_bus(&mut report) {
        Some(connection) => connection,
        None => {
            report.summary();
            std::process::exit(1);
        }
    };

    let names = match bus::names(&connection) {
        Ok(names) => names,
        Err(err) => {
            report.fail("name listing", err);
            report.summary();
            std::process::exit(1);
        }
    };

    check_portal(&mut report, &connection, &names);
    check_backends(&mut report, &names);
    check_services(&mut report, &connection, &names);
    check_secrets(&mut report, &connection, &names);
    check_environment(&mut report);

    if run_interactive {
        check_interactive(&mut report, &connection);
    }

    report.summary();
    if report.failures > 0 {
        std::process::exit(1);
    }
}

fn check_bus(report: &mut Report) -> Option<Connection> {
    report.section("bus");
    let (address, origin) = bus::address();
    report.info("address", &address);
    report.info("resolved from", origin);

    // The isolation question, and the only one a nested session can get wrong
    // without any error appearing anywhere.
    match bus::is_default_bus(&address) {
        Some(true) => report.info("identity", "the default per-user bus (not isolated)"),
        Some(false) => report.info("identity", "a private bus (isolated from the default)"),
        None => report.info("identity", "not a unix socket path — cannot classify"),
    }

    match Connection::session() {
        Ok(connection) => {
            report.ok("connect", "session bus reachable");
            Some(connection)
        }
        Err(err) => {
            report.fail("connect", err);
            None
        }
    }
}

fn check_portal(report: &mut Report, connection: &Connection, names: &bus::Names) {
    report.section("portal");
    let present = names.has(portal::DESTINATION);
    report.presence(present, true, portal::DESTINATION, names.how(portal::DESTINATION));
    if !present {
        report.info("", "nothing below can succeed — install xdg-desktop-portal");
        return;
    }

    for (interface, required) in portal::INTERFACES {
        match portal::version(connection, interface) {
            Ok(version) => report.ok(interface, format!("version {version}")),
            // Two very different failures wear the same Err here. A missing
            // interface is a portal build that simply does not carry it (or a
            // sandbox filtering it); anything else means the service is there
            // and broken, which is far more interesting.
            Err(err) => {
                let text = err.to_string();
                let detail = match text.contains("No such interface") {
                    true => String::from("not offered by this portal"),
                    false => short(&err),
                };
                report.presence(false, *required, interface, detail);
            }
        }
    }

    if let Ok(kinds) = portal::source_types(connection) {
        report.info("ScreenCast sources", kinds);
    }
}

fn check_backends(report: &mut Report, names: &bus::Names) {
    report.section("backend");
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    // Unset is a real problem, not a curiosity: the backend is chosen by
    // matching this against each backend's portal files, so with nothing to
    // match on the frontend can end up serving no backend at all.
    match desktop.is_empty() {
        true => report.warn("XDG_CURRENT_DESKTOP", "<unset> — the portal has no desktop to match on"),
        false => report.info("XDG_CURRENT_DESKTOP", &desktop),
    }

    let impls = names.with_prefix("org.freedesktop.impl.portal.desktop.");
    // A frontend with no backend is the quiet failure this whole section exists
    // for: every interface reports a healthy version and the file chooser still
    // does nothing at all.
    report.presence(!impls.is_empty(), true, "implementations", match impls.is_empty() {
        true => String::from("none installed"),
        false => impls.join(", "),
    });

    let configs = portal::configs(&desktop);
    if configs.is_empty() {
        report.warn("portals.conf", "none found — backend choice is unconfigured");
    }
    for (path, default) in configs {
        report.info(
            "portals.conf",
            format!("{} [{}]", path.display(), default.unwrap_or_else(|| String::from("no default="))),
        );
    }
}

fn check_services(report: &mut Report, connection: &Connection, names: &bus::Names) {
    report.section("session services");
    for (name, path, required, consequence) in services::SERVICES {
        if !names.has(name) {
            report.presence(false, *required, name, format!("absent — {consequence}"));
            continue;
        }
        // Registered but unable to answer is the failure a name listing hides,
        // and the one that looks like an application bug from the outside.
        match services::ping(connection, name, path) {
            Ok(()) => report.ok(name, format!("responds ({})", names.how(name))),
            Err(err) => report.presence(false, *required, name, format!("registered but not answering: {}", short(&err))),
        }
    }
}

/// The keyring, separately, because "present" and "will not interrupt you" are
/// different questions and only the second one is ever noticed.
fn check_secrets(report: &mut Report, connection: &Connection, names: &bus::Names) {
    report.section("secrets / keyring");
    if !names.has(secrets::NAME) {
        report.warn(secrets::NAME, "absent — apps needing a secret will fail or offer to create a keyring");
        return;
    }
    match services::owner_command(connection, secrets::NAME) {
        Some(owner) => report.info("implementation", owner),
        None => report.info("implementation", "activatable, not yet started"),
    }

    match secrets::collections(connection) {
        Err(err) => report.fail("collections", format!("service present but unreadable: {}", short(&err))),
        Ok(found) if found.is_empty() => {
            report.warn("collections", "none — the first app to want a secret is asked to create one")
        }
        Ok(found) => {
            let mut has_default = false;
            for collection in &found {
                let tag = match collection.is_default {
                    true => " [default]",
                    false => "",
                };
                has_default |= collection.is_default;
                let label = format!("{}{tag}", collection.label);
                // A locked DEFAULT collection is the prompt, deterministically.
                // A locked secondary one is normal and mostly harmless.
                match (collection.locked, collection.is_default) {
                    (false, _) => report.ok(&label, "unlocked"),
                    (true, true) => report.fail(&label, "LOCKED — this is what prompts you for a password"),
                    (true, false) => report.info(&label, "locked (not the default collection)"),
                }
            }
            if !has_default {
                report.warn("default alias", "unset — no collection is the one apps reach for");
            }
        }
    }

    // The cause, when the default collection is locked. Auto-unlock happens in
    // the PAM stack of whatever STARTED the session — a display manager's
    // service, typically — not in the compositor and not in y5-lock.
    match secrets::keyring_module() {
        None => report.warn("pam_gnome_keyring.so", "not installed — no PAM stack can auto-unlock"),
        Some(path) => {
            report.info("pam_gnome_keyring.so", path);
            let wired = secrets::keyring_pam_services();
            if wired.is_empty() {
                report.warn("PAM auto-unlock", "no /etc/pam.d service references it");
            }
            for (service, auth, session) in wired {
                let roles = match (auth, session) {
                    (true, true) => "auth + session (full auto-unlock)",
                    (true, false) => "auth only (unlocks, does not start the daemon)",
                    (false, true) => "session only — does NOT unlock, expect a prompt",
                    (false, false) => "password-change hook only",
                };
                report.info(format!("  /etc/pam.d/{service}").as_str(), roles);
            }
        }
    }
}

fn check_environment(report: &mut Report) {
    report.section("environment");
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    report.info("XDG_RUNTIME_DIR", match runtime.is_empty() {
        true => String::from("<unset>"),
        false => runtime.clone(),
    });

    // A WAYLAND_DISPLAY naming a socket that is not in this runtime dir means
    // apps and compositor disagree about which display they are on.
    match std::env::var("WAYLAND_DISPLAY") {
        Ok(display) if !runtime.is_empty() => {
            let path = match display.starts_with('/') {
                true => std::path::PathBuf::from(&display),
                false => std::path::PathBuf::from(&runtime).join(&display),
            };
            match path.exists() {
                true => report.ok("WAYLAND_DISPLAY", format!("{display} → {}", path.display())),
                false => report.fail("WAYLAND_DISPLAY", format!("{display} → {} does not exist", path.display())),
            }
        }
        Ok(display) => report.warn("WAYLAND_DISPLAY", format!("{display} (no runtime dir to resolve it in)")),
        Err(_) => report.warn("WAYLAND_DISPLAY", "<unset> — not a Wayland session"),
    }

    for name in ["DISPLAY", "XDG_SESSION_TYPE", "XDG_SESSION_DESKTOP", "XDG_SESSION_ID", "XDG_SEAT"] {
        let value = std::env::var(name).unwrap_or_else(|_| String::from("<unset>"));
        report.info(name, value);
    }

    // The XDG_SESSION_* family is not advertised by anything — pam_systemd puts
    // it in the environment at login and every child inherits it. A process that
    // never went through PAM (a container, a cron job, a bare ssh exec) sees none
    // of it, which says nothing whatsoever about the desktop that started it.
    // Saying so here stops an empty value from being read as a broken session.
    if std::env::var_os("XDG_SESSION_ID").is_none() {
        report.info("", "no logind session — XDG_SESSION_* comes from pam_systemd at login");
    }
}

fn check_interactive(report: &mut Report, connection: &Connection) {
    report.section("interactive");
    report.info("", "each call below opens a real window and waits for you");

    let checks: [(&str, fn(&Connection) -> zbus::Result<String>); 3] = [
        ("FileChooser.OpenFile", interactive::open_file),
        ("Screenshot.Screenshot", interactive::screenshot),
        ("ScreenCast.Start", interactive::screencast),
    ];
    for (label, call) in checks {
        println!("  ->  {label}");
        match call(connection) {
            Ok(outcome) => {
                // Cancelling is a valid answer from the user, not a broken
                // portal — the dialog appeared, which is what was being tested.
                let status = match outcome.contains("cancelled") {
                    true => Status::Warn,
                    false => Status::Ok,
                };
                report.line(status, label, outcome);
            }
            Err(err) => report.fail(label, short(&err)),
        }
    }
}

/// D-Bus errors carry the full remote message; keep the line readable.
fn short(err: &zbus::Error) -> String {
    let text = err.to_string();
    match text.char_indices().nth(90) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}
