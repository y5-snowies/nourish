/// Best-effort lookup of the current user's name for PAM.
/// Tries $USER, $LOGNAME, then `id -un` as a last resort.
pub fn current_username() -> Option<String> {
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return Some(u);
        }
    }
    if let Ok(u) = std::env::var("LOGNAME") {
        if !u.is_empty() {
            return Some(u);
        }
    }
    let output = compositor_support_library_process_child_hygiene::hygiene::command("id")
        .arg("-un")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    if name.is_empty() { None } else { Some(name) }
}
