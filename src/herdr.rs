//! Thin wrapper over the herdr CLI. Everything goes through `HERDR_BIN_PATH`.

use serde_json::Value;
use std::process::Command;

fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

/// Working directory of the focused pane. The popup is not a pane, so the
/// focused pane is always one of the user's.
pub fn focused_cwd() -> Option<String> {
    let out = Command::new(bin()).args(["pane", "list"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v["result"]["panes"]
        .as_array()?
        .iter()
        .filter(|p| p["focused"].as_bool().unwrap_or(false))
        .find_map(|p| {
            p["foreground_cwd"]
                .as_str()
                .or_else(|| p["cwd"].as_str())
                .map(str::to_owned)
        })
}

/// Open the PR popup. Size comes from the `pr` pane entry in herdr-plugin.toml.
pub fn toggle() {
    let _ = Command::new(bin())
        .args(["plugin", "pane", "open", "--plugin", "herdr-pr-pane", "--entrypoint", "pr", "--placement", "popup"])
        .output();
}
