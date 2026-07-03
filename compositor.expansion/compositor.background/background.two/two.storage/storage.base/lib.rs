//! The per-world 2D-background storage slot tokens (`BG_TWO`/`BG_TWO_MUT`) and
//! this domain's persistence wiring. Split out of `two.system` so the system
//! crate holds only system logic; the tokens live here so the `Persist` document
//! can too (they reference each other).

pub mod base;
