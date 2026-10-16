//! Imbue board — subprocess only. Spellbook does not absorb Imbue.

use std::process::Command;

pub fn set_path(text: &str) -> bool {
    Command::new("imbue")
        .args(["set", text])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn get_path() -> String {
    let Ok(out) = Command::new("imbue")
        .arg("get")
        .output()
    else {
        return String::new();
    };
    if !out.status.success() {
        return String::new();
    }
    let mut t = String::from_utf8_lossy(&out.stdout).into_owned();
    if t.ends_with('\n') {
        t.pop();
        if t.ends_with('\r') {
            t.pop();
        }
    }
    t
}
