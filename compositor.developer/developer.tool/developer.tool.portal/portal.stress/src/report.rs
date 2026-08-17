//! Output and the verdict.
//!
//! Every check prints one line and, if it is `Fail`, moves the exit code. The
//! Ok/Warn split is the whole editorial judgement in this tool: `Fail` means a
//! session that cannot open a file chooser, `Warn` means one that will work but
//! degrade (no trash in Files, no saved passwords).

use std::fmt::Display;

#[derive(Clone, Copy, PartialEq)]
pub enum Status {
    Ok,
    /// Missing, but the session still functions without it.
    Warn,
    /// Missing, and something the user will visibly try to do is now broken.
    Fail,
    /// Not a verdict — a value worth printing.
    Info,
}

impl Status {
    fn tag(self) -> &'static str {
        match self {
            Status::Ok => "OK  ",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
            Status::Info => "    ",
        }
    }
}

#[derive(Default)]
pub struct Report {
    pub failures: usize,
    pub warnings: usize,
}

impl Report {
    pub fn section(&self, title: &str) {
        println!();
        println!("── {title} {}", "─".repeat(62usize.saturating_sub(title.len())));
    }

    pub fn line(&mut self, status: Status, label: &str, detail: impl Display) {
        match status {
            Status::Fail => self.failures += 1,
            Status::Warn => self.warnings += 1,
            _ => {}
        }
        println!("  {}  {label:<34}{detail}", status.tag());
    }

    pub fn ok(&mut self, label: &str, detail: impl Display) {
        self.line(Status::Ok, label, detail);
    }

    pub fn warn(&mut self, label: &str, detail: impl Display) {
        self.line(Status::Warn, label, detail);
    }

    pub fn fail(&mut self, label: &str, detail: impl Display) {
        self.line(Status::Fail, label, detail);
    }

    pub fn info(&mut self, label: &str, detail: impl Display) {
        self.line(Status::Info, label, detail);
    }

    /// `Ok` when present, otherwise `Fail` for something load-bearing and
    /// `Warn` for something merely nice to have.
    pub fn presence(&mut self, present: bool, required: bool, label: &str, detail: impl Display) {
        let status = match (present, required) {
            (true, _) => Status::Ok,
            (false, true) => Status::Fail,
            (false, false) => Status::Warn,
        };
        self.line(status, label, detail);
    }

    pub fn summary(&self) {
        println!();
        match (self.failures, self.warnings) {
            (0, 0) => println!("all checks passed"),
            (0, w) => println!("no failures, {w} warning(s)"),
            (f, w) => println!("{f} failure(s), {w} warning(s)"),
        }
    }
}
