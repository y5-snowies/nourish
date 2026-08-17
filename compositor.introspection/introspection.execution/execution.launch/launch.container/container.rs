//! Rewrite a resolved launch so it runs INSIDE the container the window came
//! from, instead of on the host.
//!
//! This wraps whatever the plan (or a handler's synthesizer) already produced,
//! rather than being a synthesizer itself: containerisation is orthogonal to
//! app identity, so a Chrome running in a container must still get Chrome's
//! profile synthesis and then be wrapped. A `LaunchSynthesizer` is keyed by
//! handler id and would have had to displace that.
//!
//! `podman exec` does NOT inherit the caller's environment, so the request's
//! env overlay — the activation token above all, which is how the placeholder
//! recognises the window it is waiting for — has to be re-passed as `--env`
//! flags. It stays in `LaunchRequest::env` as well, harmlessly, for the podman
//! client process itself.

use compositor_introspection_execution_launch_types::types::LaunchRequest;

/// Prefix `req.argv` with the `podman exec` invocation for `container`.
///
/// `start_first` folds a `podman start` in ahead of the exec, for the case the
/// user confirmed starting a stopped container. It needs a shell because a
/// launch is one spawn and podman has no start-then-exec form; every argument
/// is quoted, so container names and paths with spaces survive.
pub fn wrap_podman(req: &mut LaunchRequest, container: &str, start_first: bool) {
    let mut exec: Vec<String> = vec!["podman".into(), "exec".into()];
    if let Some(dir) = &req.working_dir {
        exec.push("--workdir".into());
        exec.push(dir.clone());
    }
    for (k, v) in &req.env {
        exec.push("--env".into());
        exec.push(format!("{k}={v}"));
    }
    exec.push(container.to_string());
    exec.append(&mut req.argv);

    req.argv = if start_first {
        let start = shell_join(&["podman", "start", container]);
        let run = shell_join(&exec.iter().map(String::as_str).collect::<Vec<_>>());
        vec!["/bin/sh".into(), "-c".into(), format!("{start} >/dev/null 2>&1 && exec {run}")]
    } else {
        exec
    };

    // The working directory belongs to the container's filesystem, not ours —
    // it went into `--workdir` above, and applying it to the podman client
    // would make the spawn fail whenever that path doesn't exist on the host.
    req.working_dir = None;
}

fn shell_join(args: &[&str]) -> String {
    args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ")
}

/// Single-quote for `/bin/sh`: everything inside is literal, and an embedded
/// quote is closed, escaped, and reopened.
fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', r"'\''"))
}
