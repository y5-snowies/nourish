/** A structured log record as streamed from the compositor (mirrors logs.proto). */
export interface LogRecord {
  readonly elapsed_micros: number;
  readonly level: number;
  readonly crate_name: string;
  readonly function: string;
  readonly message: string;
}

/** An environment flag captured at startup. */
export interface EnvFlag {
  readonly key: string;
  readonly value: string;
}

/** Post-determined dmabuf format for one device (mirrors proto `DeviceFormat`). */
export interface DeviceFormat {
  readonly kind: string;
  readonly fourcc: string;
  readonly modifier: string;
  readonly class: string;
  readonly plane_count: number;
  readonly multiplane: boolean;
}

/** Compositor diagnostics snapshot (mirrors logs.proto `CompositorStats`). */
export interface CompositorStats {
  readonly renderer: string;
  readonly renderer_init_ok: boolean;
  readonly sync_mode: string;
  readonly output_name: string;
  readonly mode: string;
  readonly vrr_supported: boolean;
  readonly vrr_enabled: boolean;
  readonly hdr_enabled: boolean;
  readonly frames_total: number;
  readonly vblanks_total: number;
  readonly fps: number;
  readonly vblank_rate: number;
  readonly fence_synchronous: number;
  readonly fence_kms_infence: number;
  readonly fence_fallback: number;
  readonly uptime_secs: number;
  readonly env_flags: readonly EnvFlag[];
  readonly hdr_capable: boolean;
  readonly hdr_transfer: string;
  readonly hdr_max_luminance: number;
  readonly hdr_bt2020: boolean;
  readonly color_format: string;
  readonly device_formats: readonly DeviceFormat[];
}

/** Live HDR encode tuning (mirrors proto `HdrParams` / the WGSL `Tuning`). */
export interface HdrParams {
  enabled: number; // 0/1
  sdr_white_nits: number;
  max_nits: number;
  brightness: number;
  contrast: number;
  saturation: number;
  gamut: number; // 0..1
  tone_map: number; // 0/1
  transfer: number; // 0 = PQ, 1 = HLG
  gamma: number;
  exposure: number;
}

export const DEFAULT_HDR_PARAMS: HdrParams = {
  enabled: 1,
  sdr_white_nits: 203,
  max_nits: 1000,
  brightness: 1,
  contrast: 1,
  saturation: 1,
  gamut: 1,
  tone_map: 1,
  transfer: 0,
  gamma: 1,
  exposure: 1,
};

/** Level index → display name (matches the Rust `Level` discriminants). */
export const LEVELS = ["ERROR", "WARN", "INFO", "TRACE"] as const;
export type LevelName = (typeof LEVELS)[number];

/** Serializable filter state (what a preset stores). */
export interface Filters {
  levels: number[];
  crate: string;
  func: string;
  text: string;
}

export const DEFAULT_FILTERS: Filters = { levels: [0, 1, 2, 3], crate: "", func: "", text: "" };

export const LEVEL_COLOR: Readonly<Record<number, string>> = {
  0: "#ff6b6b",
  1: "#ffd166",
  2: "#8ecae6",
  3: "#8b94a0",
};

/** Format elapsed microseconds dmesg-style: `   12.345678`. */
export function formatElapsed(micros: number): string {
  const seconds = Math.floor(micros / 1_000_000);
  const rest = micros % 1_000_000;
  return `${seconds.toString().padStart(6, " ")}.${rest.toString().padStart(6, "0")}`;
}
