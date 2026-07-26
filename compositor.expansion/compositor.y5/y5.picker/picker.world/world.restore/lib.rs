// Developer logging: bring error!/warn!/info!/trace!/abort! into scope.
#[macro_use]
extern crate compositor_model_debug_instance_record;

// Startup recreation of the scene worlds restored into the picker grid from the
// `world` table, each rebuilt under its saved UUID so its per-world state +
// placeholders reload.
pub mod base;
