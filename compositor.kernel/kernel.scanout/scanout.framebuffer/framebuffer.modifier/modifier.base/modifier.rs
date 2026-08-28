//! Opt-in (Law 7) legacy-hardware support: modifier filtering for hardware
//! classes whose high-bandwidth compression modifiers fail under load.
//! DOUBLE-GATED: the `modifier-fallback` cargo feature compiles this body in,
//! and `SafetyEnable::modifier_fallback` must be set for the assembly site to
//! call it. Off = structurally absent; the modern explicit-modifier path is
//! never made worse.
//!
//! The filter is real: only LINEAR and INVALID (driver-negotiated implicit)
//! survive — the documented safe set for the affected hardware class.

#[cfg(feature = "modifier-fallback")]
pub fn filter_legacy(
    formats: smithay::backend::allocator::format::FormatSet,
) -> smithay::backend::allocator::format::FormatSet {
    let filtered: Vec<_> = formats
        .iter()
        .filter(|f| compositor_kernel_graphic_format_rule_base::rule::legacy(f.modifier))
        .copied()
        .collect();
    warn!(
        "modifier-fallback active: {} of {} render formats survive (linear/invalid only)",
        filtered.len(),
        formats.indexset().len()
    );
    filtered.into_iter().collect()
}
