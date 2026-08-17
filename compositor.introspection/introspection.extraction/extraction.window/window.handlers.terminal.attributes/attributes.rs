use compositor_introspection_extraction_window_handlers_terminal_id::id::id;
use compositor_introspection_extraction_window_hints_attribute::attribute::HintAttribute;
use compositor_introspection_extraction_window_hints_descriptor::descriptor::{AttributeDescriptor, AttributeKind};
use compositor_introspection_extraction_window_hints_id::category::AttributeCategory;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TerminalKind {
    Alacritty,
    Foot,
    GnomeTerminal,
    GnomeConsole,
    Ptyxis,
    Kitty,
    WezTerm,
    Xterm,
    Konsole,
    Unknown(String),
}

#[derive(Debug)]
pub struct TerminalKindAttr;
impl HintAttribute for TerminalKindAttr {
    type Value = TerminalKind;
    fn name() -> &'static str { "terminal.kind" }
    fn category() -> AttributeCategory { AttributeCategory::HandlerScoped(id()) }
    fn descriptor() -> AttributeDescriptor {
        AttributeDescriptor::new(
            Self::name(), "Terminal kind", Self::category(),
            AttributeKind::EnumOf(vec![
                "Alacritty", "Foot", "GnomeTerminal", "GnomeConsole",
                "Ptyxis", "Kitty", "WezTerm", "Konsole", "Xterm",
            ]),
        ).invalidated_by(&["app_id"])
    }
}

#[derive(Debug)]
pub struct LaunchCwd;
impl HintAttribute for LaunchCwd {
    type Value = PathBuf;
    fn name() -> &'static str { "terminal.launch_cwd" }
    fn category() -> AttributeCategory { AttributeCategory::HandlerScoped(id()) }
    fn descriptor() -> AttributeDescriptor {
        AttributeDescriptor::new(Self::name(), "Launch working dir", Self::category(), AttributeKind::Path)
    }
}

#[derive(Debug)]
pub struct ForegroundCwd;
impl HintAttribute for ForegroundCwd {
    type Value = PathBuf;
    fn name() -> &'static str { "terminal.foreground_cwd" }
    fn category() -> AttributeCategory { AttributeCategory::HandlerScoped(id()) }
    fn descriptor() -> AttributeDescriptor {
        AttributeDescriptor::new(Self::name(), "Foreground working dir", Self::category(), AttributeKind::Path)
    }
}

#[derive(Debug)]
pub struct Shell;
impl HintAttribute for Shell {
    type Value = String;
    fn name() -> &'static str { "terminal.shell" }
    fn category() -> AttributeCategory { AttributeCategory::HandlerScoped(id()) }
    fn descriptor() -> AttributeDescriptor {
        AttributeDescriptor::new(Self::name(), "Shell", Self::category(), AttributeKind::Text)
    }
}

/// All Terminal-scoped attribute descriptors, in display order.
pub fn descriptors() -> Vec<AttributeDescriptor> {
    vec![
        TerminalKindAttr::descriptor(),
        LaunchCwd::descriptor(),
        ForegroundCwd::descriptor(),
        Shell::descriptor(),
    ]
}
