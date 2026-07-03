import { type CSSProperties, useCallback, useEffect, useState } from "react";
import { getCompositorStats } from "./api";
import type { CompositorStats } from "./types";

function Row({ label, value }: { readonly label: string; readonly value: string }) {
  return (
    <div style={styles.row}>
      <span style={styles.label}>{label}</span>
      <span style={styles.value}>{value}</span>
    </div>
  );
}

function vrrLabel(s: CompositorStats): string {
  if (s.vrr_enabled) return "enabled";
  if (s.vrr_supported) return "supported (off)";
  return "unsupported";
}

function hdrLabel(s: CompositorStats): string {
  if (s.hdr_enabled) return "active";
  if (s.hdr_capable) return "off (display capable; COMPOSITOR_HDR=1 + Vulkan)";
  return "off (display SDR-only)";
}

export function Statistics() {
  const [stats, setStats] = useState<CompositorStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const refresh = useCallback(() => {
    setLoading(true);
    void getCompositorStats()
      .then((s) => {
        setStats(s);
        setError(null);
      })
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <div style={styles.container}>
      <div style={styles.header}>
        <h2 style={styles.title}>Statistics</h2>
        <button style={styles.button} onClick={refresh} disabled={loading}>
          {loading ? "…" : "Refresh"}
        </button>
      </div>

      {error !== null && <div style={styles.error}>⚠ {error}</div>}

      {stats === null ? (
        <div style={styles.muted}>No data yet — is the compositor running?</div>
      ) : (
        <>
          <h3 style={styles.section}>Renderer</h3>
          <Row
            label="Active renderer"
            value={`${stats.renderer} (${stats.renderer_init_ok ? "init OK" : "init FAILED"})`}
          />
          <Row label="Sync mode" value={stats.sync_mode} />

          <h3 style={styles.section}>Output</h3>
          <Row label="Output" value={stats.output_name !== "" ? stats.output_name : "(winit / nested)"} />
          <Row label="Mode" value={stats.mode !== "" ? stats.mode : "—"} />
          <Row label="VRR / adaptive sync" value={vrrLabel(stats)} />

          <h3 style={styles.section}>HDR / color</h3>
          <Row label="HDR output" value={hdrLabel(stats)} />
          <Row label="Display HDR-capable (EDID)" value={stats.hdr_capable ? "yes (PQ)" : "no"} />
          <Row label="Transfer function" value={stats.hdr_transfer} />
          <Row
            label="Display max luminance"
            value={stats.hdr_max_luminance > 0 ? `${stats.hdr_max_luminance.toFixed(0)} cd/m²` : "—"}
          />
          <Row label="Wide gamut (BT.2020)" value={stats.hdr_bt2020 ? "yes" : "no"} />
          <Row label="Color format" value={stats.color_format} />

          <h3 style={styles.section}>Throughput</h3>
          <Row label="FPS" value={stats.fps.toFixed(1)} />
          <Row label="Vblanks / sec" value={stats.vblank_rate.toFixed(1)} />
          <Row label="Frames (total)" value={String(stats.frames_total)} />
          <Row label="Vblanks (total)" value={String(stats.vblanks_total)} />
          <Row label="Uptime" value={`${stats.uptime_secs.toFixed(0)} s`} />

          <h3 style={styles.section}>Fence / sync events</h3>
          <Row label="KMS IN_FENCE" value={String(stats.fence_kms_infence)} />
          <Row label="Synchronous (device_wait_idle)" value={String(stats.fence_synchronous)} />
          <Row label="Fallback (drain)" value={String(stats.fence_fallback)} />

          <h3 style={styles.section}>GPU formats (post-determined)</h3>
          {stats.device_formats.length === 0 ? (
            <div style={styles.muted}>(no dmabuf allocated yet)</div>
          ) : (
            stats.device_formats.map((d) => (
              <Row
                key={d.kind}
                label={d.kind}
                value={`${d.fourcc} • ${d.modifier} • ${d.class} • ${d.plane_count} plane${
                  d.plane_count === 1 ? "" : "s"
                } • multiplane: ${d.multiplane ? "yes" : "no"}`}
              />
            ))
          )}

          <h3 style={styles.section}>Environment</h3>
          {stats.env_flags.length === 0 ? (
            <div style={styles.muted}>(none set)</div>
          ) : (
            stats.env_flags.map((f) => <Row key={f.key} label={f.key} value={f.value} />)
          )}
        </>
      )}
    </div>
  );
}

const styles = {
  container: {
    padding: "12px 16px",
    overflowY: "auto",
    height: "100%",
    fontFamily: "monospace",
    fontSize: 13,
    color: "#d6dbe2",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 8,
  },
  title: { margin: 0, fontSize: 16 },
  section: {
    margin: "14px 0 4px",
    fontSize: 12,
    textTransform: "uppercase",
    letterSpacing: 1,
    color: "#8b94a0",
  },
  row: {
    display: "flex",
    justifyContent: "space-between",
    gap: 16,
    padding: "2px 0",
    borderBottom: "1px solid #20262e",
  },
  label: { color: "#8ecae6" },
  value: { color: "#eaeef3", textAlign: "right", wordBreak: "break-all" },
  button: {
    background: "#2a3340",
    color: "#eaeef3",
    border: "1px solid #3a4452",
    borderRadius: 4,
    padding: "4px 12px",
    cursor: "pointer",
  },
  error: { color: "#ff6b6b", padding: "6px 0" },
  muted: { color: "#8b94a0", padding: "6px 0" },
} satisfies Record<string, CSSProperties>;
