use std::io::{self, Write};

/// Read one trimmed line from stdin, or `None` on EOF.
fn read_line() -> Option<String> {
    let mut s = String::new();
    match io::stdin().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => Some(s.trim().to_string()),
        Err(_) => None,
    }
}

/// Free-text prompt with a default.
pub fn ask(name: &str, desc: &str, default: &str) -> String {
    print!("\n{name}\n  {desc}\n  [{default}] ");
    let _ = io::stdout().flush();
    match read_line() {
        Some(s) if !s.is_empty() => s,
        _ => default.to_string(),
    }
}

/// Yes/no prompt with a default.
pub fn yes_no(name: &str, desc: &str, default: bool) -> bool {
    let d = if default { "Y/n" } else { "y/N" };
    print!("\n{name}\n  {desc}\n  [{d}] ");
    let _ = io::stdout().flush();
    match read_line() {
        Some(s) if !s.is_empty() => matches!(
            s.to_ascii_lowercase().as_str(),
            "y" | "yes" | "1" | "true" | "on"
        ),
        _ => default,
    }
}

/// Choose one of a fixed set of options, showing them all. Accepts either the
/// 1-based index or the literal value; `""` is rendered as `(empty)`.
pub fn choose(name: &str, desc: &str, options: &[&str], default: &str) -> String {
    println!("\n{name}\n  {desc}");
    for (i, opt) in options.iter().enumerate() {
        let shown = if opt.is_empty() { "(empty)" } else { opt };
        let marker = if *opt == default { " *" } else { "" };
        println!("    {}) {}{}", i + 1, shown, marker);
    }
    let def_shown = if default.is_empty() { "(empty)" } else { default };
    print!("  choice [{def_shown}] ");
    let _ = io::stdout().flush();
    match read_line() {
        Some(s) if !s.is_empty() => {
            if let Ok(n) = s.parse::<usize>() {
                if n >= 1 && n <= options.len() {
                    return options[n - 1].to_string();
                }
            }
            // Accept a literal value too.
            if options.iter().any(|o| *o == s) {
                return s;
            }
            println!("  (unrecognized '{s}', keeping default)");
            default.to_string()
        }
        _ => default.to_string(),
    }
}

/// Confirm an action (default no).
pub fn confirm(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    matches!(read_line().as_deref().map(str::to_ascii_lowercase).as_deref(), Some("y" | "yes"))
}
