//! Container detection from an already-captured `/proc` snapshot. Pure parsing
//! — no subprocess, no I/O — so it is safe on any thread, including the
//! compositor's calloop thread at window-map time.

/// Which container runtime a process was found to be running under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    Podman,
    /// `container=` said something we don't have launch support for, or the
    /// cgroup matched a container shape with no runtime marker in the env.
    Other,
}

/// A container the window's process lives inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub runtime: Runtime,
    /// Full container ID, when the cgroup carried one.
    pub id: Option<String>,
}

/// Podman's own marker: it exports `container=podman` into every container's
/// environment, and children inherit it. `container` is already on the
/// `ENV_ALLOWLIST`, so this costs nothing extra to capture.
pub fn runtime_from_env(container_var: Option<&str>) -> Option<Runtime> {
    match container_var {
        Some("podman") => Some(Runtime::Podman),
        Some(_) => Some(Runtime::Other),
        None => None,
    }
}

/// Pull a container ID out of a `/proc/<pid>/cgroup` body.
///
/// Podman writes the ID into the cgroup path under both cgroup managers:
/// - systemd:  `…/libpod-<id>.scope/container`
/// - cgroupfs: `…/libpod_parent/libpod-<id>`
///
/// Docker's `…/docker-<id>.scope` and `…/docker/<id>` shapes are recognised too
/// — the ID is just as real there; only launch support is podman-specific.
pub fn id_from_cgroup(cgroup: &str) -> Option<String> {
    for segment in cgroup.split(['/', ':']) {
        let candidate = segment
            .strip_suffix(".scope")
            .unwrap_or(segment)
            .strip_prefix("libpod-")
            .or_else(|| segment.strip_suffix(".scope").unwrap_or(segment).strip_prefix("docker-"));
        if let Some(id) = candidate {
            if is_container_id(id) {
                return Some(id.to_string());
            }
        }
        // `…/docker/<id>` and `…/libpod_parent/<id>`: a bare hex segment.
        if is_container_id(segment) {
            return Some(segment.to_string());
        }
    }
    None
}

/// Container IDs are 64 lowercase hex chars. Requiring the full width keeps
/// unrelated hex-looking cgroup segments (slice names, uids) from matching.
fn is_container_id(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Combine the two signals. Returns `None` for a process that isn't in a
/// container: neither the env marker nor a container-shaped cgroup.
pub fn detect(cgroup: Option<&str>, container_var: Option<&str>) -> Option<Container> {
    let id = cgroup.and_then(id_from_cgroup);
    let runtime = runtime_from_env(container_var);
    match (runtime, &id) {
        (None, None) => None,
        (runtime, _) => Some(Container { runtime: runtime.unwrap_or(Runtime::Other), id }),
    }
}
