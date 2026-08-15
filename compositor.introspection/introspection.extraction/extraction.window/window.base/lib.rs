//! Façade for the window-extraction crates: pure data capture from a live
//! Wayland window (process tree + inferred hints). Re-exports the full old
//! module tree; the implementation lives in the sibling `window.*` crates.
//!
//! Entry points: [`extract_meta`] (window must be alive) and
//! [`extract_hints`] (pure data + filesystem reads).

pub mod desktop {
    pub use compositor_introspection_extraction_window_desktop_entry::desktop::*;
    pub use compositor_introspection_extraction_window_desktop_search::desktop::*;
}

pub mod handler {
    pub use compositor_introspection_extraction_window_handler_registry::registry;
    pub use compositor_introspection_extraction_window_handler_traits::traits;
    pub use self::registry::HandlerRegistry;
    pub use self::traits::{AppHandler, DetectResult};
}

pub mod handlers {
    pub use compositor_introspection_extraction_window_handlers::handlers::*;
}

pub mod hints {
    pub use compositor_introspection_extraction_window_hints_attribute::attribute;
    pub mod attributes {
        pub use compositor_introspection_extraction_window_hints_attributes_identity::attributes::*;
        pub use compositor_introspection_extraction_window_hints_attributes_identity_more::attributes::*;
        pub use compositor_introspection_extraction_window_hints_attributes_launch::attributes::*;
    }
    pub use compositor_introspection_extraction_window_hints_id::{category, handler_id};
    pub use compositor_introspection_extraction_window_hints_descriptor::descriptor;
    pub mod extract {
        pub use compositor_introspection_extraction_window_hints_extract::extract::*;
        pub use compositor_introspection_extraction_window_hints_extract_entry::extract::*;
    }
    pub mod inferred {
        pub use compositor_introspection_extraction_window_hints_inferred::inferred::*;
        pub use compositor_introspection_extraction_window_hints_inferred_raw::inferred::*;
    }
    pub use compositor_introspection_extraction_window_hints_source::source;
    pub use compositor_introspection_extraction_window_hints_values::{sandbox, values};

    pub use self::attribute::{HintAttribute, TypedHint};
    pub use self::category::AttributeCategory;
    pub use self::descriptor::{AttributeDescriptor, AttributeKind};
    pub use self::extract::extract_base_hints;
    pub use self::handler_id::HandlerId;
    pub use self::inferred::{InferredHints, RawAlternative, RawHintView};
    pub use self::sandbox::parse_sandbox;
    pub use self::source::{Confidence, HintSource, SourceMethod};
    pub use self::values::{EnvPair, IconPixels, SandboxIdentity, ToplevelIcon};
}

pub mod icon {
    pub use compositor_introspection_extraction_window_icon::icon::*;
    /// The live-surface reader for `xdg_toplevel_icon_v1` (name + buffers).
    pub use compositor_introspection_extraction_window_icon_toplevel::icon_toplevel as toplevel;
}

pub mod meta {
    pub use compositor_introspection_extraction_window_meta_types::{env, types};
    pub mod proc {
        pub use compositor_introspection_extraction_window_meta_proc_read::proc::*;
        pub use compositor_introspection_extraction_window_meta_proc_tree::proc::*;
    }
    pub mod wayland {
        pub use compositor_introspection_extraction_window_meta_wayland::wayland::*;
        pub use compositor_introspection_extraction_window_meta_wayland_surface::wayland::*;
    }

    pub use self::env::ENV_ALLOWLIST;
    pub use self::proc::{
        extract_full_tree, extract_meta_for_pid, extract_tree, refresh_meta_from_pid,
        walk_parents, DEFAULT_CHILD_DEPTH, DEFAULT_PARENT_STEPS,
    };
    pub use self::types::{Meta, MetaNode};
    pub use self::wayland::{extract_from_window, extract_node_from_window, extract_surface_meta};
}

pub use handler::{AppHandler, DetectResult, HandlerRegistry};
pub use handlers::default_registry;
pub use hints::attributes;
pub use hints::{
    AttributeCategory, AttributeDescriptor, AttributeKind, Confidence, EnvPair, HandlerId,
    HintAttribute, HintSource, IconPixels, InferredHints, RawAlternative, SandboxIdentity,
    SourceMethod, ToplevelIcon, TypedHint,
};
pub use meta::{Meta, MetaNode, refresh_meta_from_pid};

/// Extract a [`MetaNode`] from a live Wayland window. **Must be called while
/// the window and its process are alive.**
pub use meta::wayland::extract_meta;

/// Extract the WAYLAND half of a [`Meta`] — surface identity + client
/// credentials, no `/proc` walk. Cheap enough to run over every window at once.
pub use meta::wayland::extract_surface_meta;

/// Extract [`InferredHints`] from a (possibly stale) [`MetaNode`]. Pure data
/// + filesystem reads; does not touch the window or `/proc`.
pub use handler::registry::extract_hints;

/// [`extract_hints`] with the extra context only a live window can supply: its
/// `xdg_toplevel_icon_v1` icon. Read it with [`icon::toplevel::read`].
pub use handler::registry::extract_hints_with;
