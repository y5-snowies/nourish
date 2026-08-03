use compositor_background_two_state_base::state::Two;
use compositor_support_system_storage_token_base::base::{Token, TokenMut};

/// The per-world 2D parallax background slot.
pub static BG_TWO: Token<Two> = Token::new();
/// TRANSITIONAL pub: lock/capture still mutate the instance directly.
pub static BG_TWO_MUT: TokenMut<Two> = TokenMut::new(&BG_TWO);

/// This world's persisted background: the shader override + its edited variable
/// values keyed by `@prop` name (robust to slot/order changes).
#[derive(serde::Serialize, serde::Deserialize, PartialEq)]
struct BackgroundPersisted {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shader: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    params: Vec<(String, f32)>,
    #[serde(default, skip_serializing_if = "is_false")]
    invert_pan_x: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    invert_pan_y: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    srgb: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    optimized: bool,
}

fn is_false(b: &bool) -> bool { !*b }

/// Whether a persisted selection still names something that exists.
///
/// The RENDER path already falls back to the stock parallax on its own — a
/// missing source finds no bundle file either and `shader.load` returns nothing.
/// What it does not do is fix the SLOT, and a stale id there makes the settings
/// panel lie: it highlights nothing in the picker, reports "no variables" while
/// the parallax's six are live, and greys out the Optimized toggle even though
/// the shader actually rendering has an optimized twin. Rewriting the id to
/// `None` on load is what keeps the panel describing what is on screen.
///
/// Only `builtin:` ids are checked, against the live table — so a built-in that
/// is dropped or renamed is handled with no list to maintain, which is what a
/// hard-coded roll of removed ids was really standing in for. A bundle
/// selection is deliberately left alone: its folder may simply not be mounted
/// yet, and forgetting the user's choice for that would be worse than a
/// temporarily wrong panel.
fn selection_exists(id: &str) -> bool {
    !id.starts_with(compositor_background_two_shader_builtin::BUILTIN_PREFIX)
        || compositor_background_two_shader_builtin::source(id).is_some()
}

/// Transforms the per-world `Two` slot to/from its persisted form (a single
/// value, so `Persist`/`y5_persist!` — not the collection `Document`).
struct BackgroundPersist;
impl compositor_support_system_persist_trait_base::base::Persist for BackgroundPersist {
    type Live = Two;
    type Persisted = BackgroundPersisted;
    const KEY: &'static str = "world.background";
    const CURRENT_VERSION: u32 = 1;
    fn to_persisted(live: &Two) -> BackgroundPersisted {
        BackgroundPersisted {
            shader: live.background_shader.clone(),
            params: live.params.clone(),
            invert_pan_x: live.invert_pan_x,
            invert_pan_y: live.invert_pan_y,
            srgb: live.srgb,
            optimized: live.optimized,
        }
    }
    fn from_persisted(p: BackgroundPersisted) -> Two {
        let mut two = Two::new();
        two.background_shader = p.shader.filter(|s| selection_exists(s));
        two.params = p.params;
        two.invert_pan_x = p.invert_pan_x;
        two.invert_pan_y = p.invert_pan_y;
        two.srgb = p.srgb;
        two.optimized = p.optimized;
        two
    }
}
compositor_support_system_persist_trait_base::y5_persist!(
    BACKGROUND_PERSIST, BackgroundPersist, BG_TWO, BG_TWO_MUT
);

/// This domain's persist entries, returned by the system's `persist()`.
pub static BACKGROUND_PERSISTS: &[&compositor_support_system_persist_entry_base::base::PersistEntry] =
    &[&BACKGROUND_PERSIST];
