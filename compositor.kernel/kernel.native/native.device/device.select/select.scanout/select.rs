//! Split render/scanout (PRIME) resolution: pick the KMS-capable SCANOUT card,
//! anchored to the configured RENDER node, and compute the copy route between them.
//! Desktop: the render card is itself KMS-capable, so it IS the scanout card (route
//! None) — byte-identical to the pre-split path. Only a render-only GPU (Tegra
//! `nvgpu`) diverges into DmabufCopy. A selected card with no KMS is a verbose panic,
//! never a silent fallback. Route/role compare at CARD (Primary) granularity — a
//! GPU's render node and card node have different dev_t, so comparing render-vs-card
//! would falsely read "different device" on every desktop.

use compositor_kernel_drm_device_capability_base::capability::{self, KmsCapability};
use compositor_kernel_drm_device_open_base::open::wrap_fd;
use compositor_kernel_gpu_topology_route_base::route::{self, CopyRoute};
use compositor_kernel_seat_interface_open_base::open::open as seat_open;
use smithay::backend::drm::{DrmDeviceFd, DrmNode, NodeType};
use smithay::backend::session::Session;
use std::path::{Path, PathBuf};

pub struct Resolved {
    /// The render node the compositor composites on (wgpu pin + render GBM).
    pub render: DrmNode,
    /// The KMS card that scans out (opened as the smithay DrmDevice).
    pub scanout: DrmNode,
    pub scanout_path: PathBuf,
    /// The already-opened, KMS-probed scanout fd (reused for DrmDevice + GBM — no
    /// second open on the common desktop path).
    pub scanout_fd: DrmDeviceFd,
    pub cap: KmsCapability,
    /// None when render card == scanout card (desktop); DmabufCopy on a split GPU.
    pub route: CopyRoute,
}

/// dev_t of the PRIMARY (card) node behind any card/render path.
fn card_id(path: &Path) -> Option<u64> {
    Some(card_node(path)?.dev_id())
}

/// The PRIMARY (card) node behind a card/render path (falls back to the node itself).
fn card_node(path: &Path) -> Option<DrmNode> {
    let n = DrmNode::from_path(path).ok()?;
    Some(n.node_with_type(NodeType::Primary).and_then(|r| r.ok()).unwrap_or(n))
}

/// The RENDER node behind a card/render path (falls back to the node itself).
fn render_of(path: &Path) -> Option<DrmNode> {
    let n = DrmNode::from_path(path).ok()?;
    Some(n.node_with_type(NodeType::Render).and_then(|r| r.ok()).unwrap_or(n))
}

fn card_path_for(cards: &[(u64, PathBuf)], want: u64) -> Option<&PathBuf> {
    cards.iter().find(|(id, _)| *id == want).map(|(_, p)| p)
}

fn open_probe<S: Session>(session: &mut S, path: &Path) -> (DrmDeviceFd, KmsCapability)
where
    S::Error: std::fmt::Debug,
{
    let fd = wrap_fd(seat_open(session, path));
    let cap = capability::probe(&fd);
    (fd, cap)
}

/// One line of the diagnostic card table used in the verbose panic.
fn describe(path: &Path, cap: &KmsCapability) -> String {
    format!(
        "    {:<16} driver {:<12} KMS {}  (crtcs={}, connectors={}, connected={})",
        path.display(),
        cap.driver.clone().unwrap_or_else(|| "?".into()),
        if cap.is_scanout_capable() { "OK" } else { "no" },
        cap.crtcs,
        cap.connectors,
        cap.connected,
    )
}

fn finish(
    render: DrmNode,
    render_card_node: DrmNode,
    path: PathBuf,
    fd: DrmDeviceFd,
    cap: KmsCapability,
) -> Resolved {
    let scanout = card_node(&path)
        .unwrap_or_else(|| abort!("scanout card {path:?} has no primary node"));
    let route = route::route(render_card_node, scanout);
    info!("scanout resolved: {path:?} (driver {:?}); route={route:?}", cap.driver);
    Resolved { render, scanout, scanout_path: path, scanout_fd: fd, cap, route }
}

pub fn resolve<S: Session>(
    session: &mut S,
    cards: &[(u64, PathBuf)],
    render_pref: Option<&Path>,
    heuristic: Option<&Path>,
    scanout_pref: Option<&Path>,
    render_fallback: bool,
    scan_fallback: bool,
) -> Resolved
where
    S::Error: std::fmt::Debug,
{
    // --- Render node: anchored to settings at CARD (dev_id) granularity. STRICT — if
    // the configured render_node isn't a present card, panic UNLESS `render_discovery`,
    // which re-enables the heuristic→first-card fallback (never a silent substitution).
    let render_card_path: PathBuf = match render_pref
        .and_then(card_id)
        .and_then(|id| card_path_for(cards, id))
    {
        Some(p) => p.clone(),
        None => {
            if !render_fallback {
                abort!(
                    "FATAL: render_node {render_pref:?} is not a present DRM card on the seat. \
                     Point render_node at an available card, or add the `render_discovery` mode \
                     token to allow heuristic/first-card fallback. (strict: no silent substitution)"
                );
            }
            heuristic
                .and_then(card_id)
                .and_then(|id| card_path_for(cards, id))
                .or_else(|| cards.first().map(|(_, p)| p))
                .cloned()
                .unwrap_or_else(|| abort!("no DRM cards on seat — cannot select a render GPU"))
        }
    };
    let render = render_of(&render_card_path)
        .unwrap_or_else(|| abort!("render card {render_card_path:?} has no resolvable DRM node"));
    let render_card_node = card_node(&render_card_path).unwrap();
    let render_card = render_card_node.dev_id();

    // --- Explicit scanout override. KMS-capable → use it. Not KMS → panic UNLESS
    //     `scanout_discovery`, which lets a bad explicit choice fall THROUGH to
    //     auto-discovery instead of aborting.
    if let Some(p) = scanout_pref {
        let (fd, cap) = open_probe(session, p);
        if cap.is_scanout_capable() {
            return finish(render, render_card_node, p.to_path_buf(), fd, cap);
        }
        if !scan_fallback {
            abort!(
                "FATAL: explicit scanout_node {p:?} (driver {:?}) has no KMS/modeset \
                 capability (crtcs={}, connectors={}). Point it at a KMS-capable card, or add \
                 the `scanout_discovery` mode token to fall through to auto-discovery.",
                cap.driver, cap.crtcs, cap.connectors
            );
        }
        warn!(
            "explicit scanout_node {p:?} is not KMS-capable; `scanout_discovery` set → \
             falling through to auto-discovery"
        );
    }

    // --- Auto-discover. #3 first: render card itself, if KMS-capable (desktop — the
    //     stable path, needs no token).
    let (render_fd, render_cap) = open_probe(session, &render_card_path);
    if render_cap.is_scanout_capable() {
        return finish(render, render_card_node, render_card_path, render_fd, render_cap);
    }

    // #2: render card is render-only (e.g. Tegra nvgpu). STRICT — discovering a
    // *different* KMS card requires `scanout_discovery`; otherwise panic.
    if !scan_fallback {
        abort!(
            "FATAL: render_node {render_card_path:?} is render-only (no KMS) and no KMS-capable \
             `scanout_node` is set. Set `scanout_node` to your display card, or add the \
             `scanout_discovery` mode token to auto-discover one. (strict: no silent discovery)"
        );
    }
    // Probe the other cards and
    // pick a KMS-capable one, anchored to the render card: rank by connected
    // displays desc (the card actually driving output), then card path asc.
    let mut probed: Vec<(PathBuf, DrmDeviceFd, KmsCapability)> = Vec::new();
    for (id, path) in cards {
        if *id == render_card {
            continue; // render card already probed above
        }
        let (fd, cap) = open_probe(session, path);
        probed.push((path.clone(), fd, cap));
    }
    let mut order: Vec<usize> =
        (0..probed.len()).filter(|&i| probed[i].2.is_scanout_capable()).collect();
    order.sort_by(|&a, &b| {
        probed[b].2.connected.cmp(&probed[a].2.connected).then(probed[a].0.cmp(&probed[b].0))
    });

    match order.first().copied() {
        Some(idx) => {
            if order.len() > 1 {
                let others: Vec<&PathBuf> = order.iter().skip(1).map(|&i| &probed[i].0).collect();
                warn!(
                    "scanout: {} KMS cards; chose {:?} (connected={}) — others {others:?}",
                    order.len(),
                    probed[idx].0,
                    probed[idx].2.connected
                );
            }
            warn!(
                "split render/scanout: render {render_card_path:?} is render-only, \
                 scanning out on {:?}",
                probed[idx].0
            );
            let (path, fd, cap) = probed.swap_remove(idx); // own the winner; rest drop (close)
            finish(render, render_card_node, path, fd, cap)
        }
        None => {
            let table: String = std::iter::once((render_card_path.clone(), render_cap))
                .chain(probed.into_iter().map(|(p, _, c)| (p, c)))
                .map(|(p, c)| describe(&p, &c))
                .collect::<Vec<_>>()
                .join("\n");
            abort!(
                "FATAL: no KMS/modeset-capable scanout card found on the seat.\n\
                 render_node (settings) = {render_card_path:?} — render-only (no KMS).\n\
                 Cards probed:\n{table}\n\
                 -> This looks like a hybrid/PRIME system; scanout needs a KMS card.\n\
                    If none is listed, set `nvidia-drm.modeset=1` on the kernel cmdline\n\
                    (or the equivalent for your display driver) so a KMS DRM device exists."
            );
        }
    }
}
