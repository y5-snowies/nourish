//! The canonical complete starting settings now live ONCE in
//! `config.base::default_settings` — shared with the installer's seed so the two can
//! never drift, and complete across the full schema (the compositor requires every
//! field). Re-exported here so the rest of the tool keeps calling
//! `template::default_settings()`.

pub use compositor_model_environment_config_base::base::default_settings;
