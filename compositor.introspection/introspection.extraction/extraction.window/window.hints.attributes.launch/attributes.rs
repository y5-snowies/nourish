//! Generic exec primitives, in `AttributeCategory::Launch`. Used as fallback
//! when no handler synthesis is active.

use compositor_introspection_extraction_window_hints_attribute::attribute::HintAttribute;
use compositor_introspection_extraction_window_hints_descriptor::descriptor::{AttributeDescriptor, AttributeKind};
use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;
use compositor_introspection_extraction_window_hints_values::values::EnvPair;
use std::path::PathBuf;

/// Path to the binary that runs this app.
#[derive(Debug)]
pub struct ExecProgram;
impl HintAttribute for ExecProgram {
    type Value = PathBuf;
    fn name() -> &'static str { "exec_program" }
    fn category() -> AttributeCategory { AttributeCategory::Launch }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Program", Self::category(), AttributeKind::Path).invalidated_by(&["terminal.kind"]) }
}

/// argv tail (without the program), as a single value.
#[derive(Debug)]
pub struct ExecArgs;
impl HintAttribute for ExecArgs {
    type Value = Vec<String>;
    fn name() -> &'static str { "exec_args" }
    fn category() -> AttributeCategory { AttributeCategory::Launch }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Arguments", Self::category(), AttributeKind::StringList) }
}

/// Working directory the launched process should start in.
#[derive(Debug)]
pub struct WorkingDirectory;
impl HintAttribute for WorkingDirectory {
    type Value = PathBuf;
    fn name() -> &'static str { "working_directory" }
    fn category() -> AttributeCategory { AttributeCategory::Launch }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Working directory", Self::category(), AttributeKind::Path) }
}

/// All env vars the launched process should inherit/override, as one list.
#[derive(Debug)]
pub struct EnvOverlay;
impl HintAttribute for EnvOverlay {
    type Value = Vec<EnvPair>;
    fn name() -> &'static str { "env_overlay" }
    fn category() -> AttributeCategory { AttributeCategory::Launch }
    fn descriptor() -> AttributeDescriptor { AttributeDescriptor::new(Self::name(), "Environment variables", Self::category(), AttributeKind::Custom("env_pair_list")) }
}
