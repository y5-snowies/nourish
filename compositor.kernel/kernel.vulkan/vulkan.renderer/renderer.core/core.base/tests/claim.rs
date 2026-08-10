//! Where the ownership claim COMES FROM, which is the part that was wrong.
//!
//! `Own` decides whether the engine draws the world band at all, so a claim the
//! bundle did not make for this frame blanks every window on screen. The claim now
//! travels ON THE FRAME — a field on the graph's op when the bundle ran inline, and
//! on the background texture's `ElementMeta` when it ran on the worker — so a frame
//! with no background element in it cannot claim anything.
//!
//! It used to travel through a process-global atomic, published while the scene was
//! built and consumed by the submit that followed. That pairing is false under
//! uncapped pacing (many composites per background frame) and with more than one
//! render target, and a submit taking a claim meant for another is exactly the
//! reported fault: windows gone, reappearing wherever something perturbs the
//! timing. `claim()` taking its answer as an ARGUMENT rather than reading a slot is
//! what makes that unrepresentable, and this file is what keeps it that way.
//!
//! These are pure functions of their arguments — no device, no globals — which is
//! itself the property under test: if `claim` ever reads shared state again, it
//! stops being callable like this.

use compositor_pipeline_abi_worldset_base::base::Own;
use compositor_kernel_vulkan_renderer_core_base::renderer::worldset::claim;

/// The failure this exists to prevent, stated directly: a frame carrying no claim
/// must not suppress the band. There is no "previous" claim to inherit, because
/// there is nowhere for one to live.
#[test]
fn a_frame_with_no_claim_does_not_suppress_the_band() {
    let c = claim(None, false);
    assert_eq!(c.own(), Own::None);
    assert!(!c.suppresses_band(), "a frame with no background element must draw the world band");
}

/// Both ownership modes survive, and both suppress. `Windows` decaying to `None`
/// would draw client windows twice; `World` decaying would draw the whole band
/// twice.
#[test]
fn every_mode_suppresses_the_band_it_claims() {
    for own in [Own::Windows, Own::World] {
        let c = claim(Some(own), false);
        assert_eq!(c.own(), own);
        assert!(c.suppresses_band(), "{own:?} must take the band it claimed");
    }
}

/// A claim is a pure function of THIS frame's argument, so the same call repeated
/// gives the same answer.
///
/// This is the regression guard. The old global was consumed by whichever submit
/// read it first, so a second read in the same frame — a second output, a second
/// pane, a composite that arrived before the next scene build — silently got
/// `None` and drew the band the bundle had already been told it owned.
#[test]
fn a_claim_is_not_consumed_by_reading_it() {
    for _ in 0..4 {
        assert_eq!(claim(Some(Own::World), false).own(), Own::World);
    }
}

/// `whole_band` widens the SET the shader is handed without changing who draws.
/// `Own::World` implies it; a bundle that only re-composites what it finds
/// (`glass`) asks for it while leaving the band to the engine.
#[test]
fn whole_band_is_membership_not_ownership() {
    let engine_wide = claim(None, true);
    assert!(engine_wide.whole_band, "a whole-band requirement widens the set");
    assert!(!engine_wide.suppresses_band(), "…without taking the band from the engine");

    assert!(claim(Some(Own::World), false).whole_band, "owning the world implies its set");
    assert!(!claim(Some(Own::Windows), false).whole_band, "owning windows does not");
}
