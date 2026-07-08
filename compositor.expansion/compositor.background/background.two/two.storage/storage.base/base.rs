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
}

fn is_false(b: &bool) -> bool { !*b }

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
        }
    }
    fn from_persisted(p: BackgroundPersisted) -> Two {
        let mut two = Two::new();
        two.background_shader = p.shader;
        two.params = p.params;
        two.invert_pan_x = p.invert_pan_x;
        two.invert_pan_y = p.invert_pan_y;
        two.srgb = p.srgb;
        two
    }
}
compositor_support_system_persist_trait_base::y5_persist!(
    BACKGROUND_PERSIST, BackgroundPersist, BG_TWO, BG_TWO_MUT
);

/// This domain's persist entries, returned by the system's `persist()`.
pub static BACKGROUND_PERSISTS: &[&compositor_support_system_persist_entry_base::base::PersistEntry] =
    &[&BACKGROUND_PERSIST];
