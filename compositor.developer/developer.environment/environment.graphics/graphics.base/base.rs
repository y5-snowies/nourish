//! Shared anti-aliasing / graphics config for the world anti-aliasing.
//!
//! One serde struct ([`GraphicsAaConfig`]) persisted in `preferences.json` and
//! edited from the settings "Graphics" tab. Because the kernel renderer cannot
//! read live preferences (it only sees the frozen startup config), the compositor
//! also pushes the latest config into a process-global here ([`set`]); the Vulkan
//! renderer reads it every frame ([`get`]) together with the live world zoom and
//! evaluates each knob for the current zoom via [`GraphicsAaConfig::effective`].

use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Which anti-aliasing technique the textured (window + iced) arm uses.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AaMethod {
    /// No AA — the plain composite path.
    Off,
    /// In-shader N×N supersample (+ optional sharpen). No mip chain.
    Ssaa,
    /// Trilinear mip sampling (+ LOD bias, + optional sharpen).
    Trilinear,
    /// Anisotropic mip sampling.
    Anisotropic,
}

impl AaMethod {
    pub const ALL: [AaMethod; 4] = [
        AaMethod::Off,
        AaMethod::Ssaa,
        AaMethod::Trilinear,
        AaMethod::Anisotropic,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AaMethod::Off => "Off",
            AaMethod::Ssaa => "Supersample (SSAA)",
            AaMethod::Trilinear => "Trilinear mips",
            AaMethod::Anisotropic => "Anisotropic",
        }
    }

    /// Samples from a generated mip chain (trilinear/aniso).
    pub fn needs_mips(self) -> bool {
        matches!(self, AaMethod::Trilinear | AaMethod::Anisotropic)
    }
}

impl Default for AaMethod {
    fn default() -> Self {
        AaMethod::Off
    }
}

/// A single tunable knob whose value scales with how far the view is zoomed out:
/// `value = base + per_zoom · zoom_out`, where `zoom_out = 1/zoom − 1` (clamped).
/// So `per_zoom` is "how much this grows per unit of minification past 1×".
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct ZoomKnob {
    pub base: f32,
    pub per_zoom: f32,
}

impl ZoomKnob {
    pub const fn new(base: f32, per_zoom: f32) -> Self {
        Self { base, per_zoom }
    }
    pub fn eval(&self, zoom_out: f32) -> f32 {
        self.base + self.per_zoom * zoom_out
    }
}

impl Default for ZoomKnob {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

/// The full anti-aliasing configuration.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct GraphicsAaConfig {
    pub method: AaMethod,
    /// AA runs only while the world zoom is below this (1.0 == the moment you
    /// zoom out at all; >1 also covers slight zoom-in).
    pub activate_below_zoom: f32,
    /// Upper clamp on `zoom_out` used for weighting, so knobs don't run away at
    /// extreme zoom-out.
    pub max_zoom_out: f32,
    /// SSAA taps per axis (effective value rounded, clamped 1..=32).
    pub taps: ZoomKnob,
    /// SSAA footprint spread (≥ 1 over-blurs).
    pub spread: ZoomKnob,
    /// Unsharp-mask amount applied after the downsample (crisper text/edges).
    pub sharpen: ZoomKnob,
    /// Trilinear mip LOD bias (negative → sharper/higher-res mip).
    pub lod_bias: ZoomKnob,
    /// Max anisotropy (1..=16). Static (a sampler property, not zoom-weighted).
    pub aniso: f32,
    /// FSR EASU (edge-adaptive spatial upscale) — an INDEPENDENT toggle, separate
    /// from `method`. A magnification filter: reconstructs sharp edges when a
    /// surface is drawn larger than its buffer (zoomed in, low-res/fractional
    /// clients). Composes with `rcas` (the canonical FSR1 EASU→RCAS chain).
    pub easu: bool,
    /// FSR RCAS (robust contrast-adaptive sharpen) — an INDEPENDENT toggle. When
    /// `easu` is also on, RCAS sharpens the EASU-reconstructed image; otherwise
    /// it sharpens the plain (bilinear) source.
    pub rcas: bool,
    /// FSR RCAS sharpening strength (0..=1; 0 = none, higher = sharper). Unlike
    /// the AA knobs (weighted by zoom-OUT), this ramps on the zoom-IN axis.
    pub rcas_sharpen: ZoomKnob,
    /// FSR gate: EASU/RCAS run only while the world zoom is ABOVE this (i.e. when
    /// zoomed IN / magnifying). 1.0 == the moment you zoom past 100%. This is the
    /// magnification counterpart to the AA method's `activate_below_zoom`.
    pub fsr_activate_above_zoom: f32,
    /// Upper clamp on `zoom_in` used for the RCAS zoom-in weighting.
    pub fsr_max_zoom_in: f32,
}

impl GraphicsAaConfig {
    /// Hard ceiling on effective SSAA taps per axis (→ `MAX_TAPS²` samples per
    /// fragment). A production safety limit independent of the UI slider range.
    pub const MAX_TAPS: u32 = 8;

    pub const DEFAULT: GraphicsAaConfig = GraphicsAaConfig {
        method: AaMethod::Off,
        activate_below_zoom: 1.0,
        max_zoom_out: 16.0,
        taps: ZoomKnob::new(4.0, 1.0),
        spread: ZoomKnob::new(1.0, 0.0),
        sharpen: ZoomKnob::new(0.5, 0.15),
        lod_bias: ZoomKnob::new(0.0, -0.2),
        aniso: 16.0,
        easu: false,
        rcas: false,
        rcas_sharpen: ZoomKnob::new(0.4, 0.0),
        fsr_activate_above_zoom: 1.0,
        fsr_max_zoom_in: 8.0,
    };

    /// Evaluate the knobs for the current world `zoom` (1.0 == 100%).
    pub fn effective(&self, zoom: f32) -> EffectiveAa {
        // The AA method (SSAA/trilinear/aniso) gates on zoom-OUT (minification);
        // the FSR filters (EASU/RCAS) gate on zoom-IN (magnification). Each side
        // has its own threshold + weight clamp. The composite pass runs if either
        // side wants to.
        let aa_on = self.method != AaMethod::Off && zoom < self.activate_below_zoom;
        let fsr_gate = zoom > self.fsr_activate_above_zoom;
        let easu = self.easu && fsr_gate;
        let rcas = self.rcas && fsr_gate;
        let active = aa_on || easu || rcas;
        let zoom_out = ((1.0 / zoom.max(1e-4)) - 1.0).clamp(0.0, self.max_zoom_out.max(0.0));
        let zoom_in = (zoom - 1.0).clamp(0.0, self.fsr_max_zoom_in.max(0.0));
        let taps = match self.method {
            // Per-axis taps; the effective count is capped to `MAX_TAPS` so a
            // heavy config can't request MAX_TAPS² samples/fragment and stall
            // weaker GPUs.
            AaMethod::Ssaa => self.taps.eval(zoom_out).round().clamp(1.0, Self::MAX_TAPS as f32) as u32,
            // Trilinear/aniso run their own kernels (single "tap").
            _ => 1,
        };
        EffectiveAa {
            active,
            aa_on,
            method: self.method,
            taps,
            spread: self.spread.eval(zoom_out).max(0.0),
            sharpen: self.sharpen.eval(zoom_out).max(0.0),
            lod_bias: self.lod_bias.eval(zoom_out),
            aniso: self.aniso.clamp(1.0, 16.0),
            easu,
            rcas,
            rcas_sharpen: self.rcas_sharpen.eval(zoom_in).clamp(0.0, 1.0),
        }
    }
}

impl Default for GraphicsAaConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The per-frame, zoom-resolved values the renderer actually uses.
#[derive(Clone, Copy, Debug)]
pub struct EffectiveAa {
    /// Any part of the composite AA pass wants to run (AA method OR EASU OR RCAS).
    pub active: bool,
    /// The AA method (SSAA/trilinear/aniso) is running for the current zoom.
    pub aa_on: bool,
    pub method: AaMethod,
    pub taps: u32,
    pub spread: f32,
    pub sharpen: f32,
    pub lod_bias: f32,
    pub aniso: f32,
    pub easu: bool,
    pub rcas: bool,
    pub rcas_sharpen: f32,
}

/// Process-global config, pushed by the compositor and read by the renderer.
static CONFIG: RwLock<GraphicsAaConfig> = RwLock::new(GraphicsAaConfig::DEFAULT);

/// Latest config (cheap uncontended read — called per frame by the renderer).
pub fn get() -> GraphicsAaConfig {
    CONFIG.read().map(|c| *c).unwrap_or(GraphicsAaConfig::DEFAULT)
}

/// Replace the live config. Called at startup (from `preferences.json`) and on
/// every settings edit.
pub fn set(cfg: GraphicsAaConfig) {
    if let Ok(mut c) = CONFIG.write() {
        *c = cfg;
    }
}
