// TEMPORARY — this should move to the y5 crates: the built-in worlds are y5's, and
// only the persist loader's need to name them without depending back on the world
// manager keeps the constants down here for now.
//
// The fixed identities of the built-in worlds. Constants only — no dependency on
// any compositor crate, so every layer (the world manager that builds them, the
// persist loader that must not mistake one for a record) can name them without
// inverting the graph.
pub mod base;
