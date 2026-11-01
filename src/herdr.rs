//! Thin wrapper over the herdr CLI. Everything goes through `HERDR_BIN_PATH`.

use serde_json::Value;
use std::process::Command;

fn bin() -> String {
    std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into())
}

/// Working directory of the pane the user is looking at, ignoring our own pane.
pub fn focused_cwd(allow_sibling: bool) -> Option<String> {
    let out = Command::new(bin()).args(["pane", "list"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let own = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let panes = v["result"]["panes"].as_array()?;
    let cwd_of = |p: &Value| {
        p["foreground_cwd"]
            .as_str()
            .or_else(|| p["cwd"].as_str())
            .map(str::to_owned)
    };
    let others = || panes.iter().filter(|p| p["pane_id"].as_str().unwrap_or("") != own);
    if let Some(cwd) = others().filter(|p| p["focused"].as_bool().unwrap_or(false)).find_map(cwd_of) {
        return Some(cwd);
    }
    if !allow_sibling {
        return None;
    }
    // Our own pane has focus: fall back to a sibling in the same tab, but only
    // before we have shown anything (the caller keeps the last context otherwise).
    let tab = panes.iter().find(|p| p["pane_id"].as_str() == Some(own.as_str()))?["tab_id"].clone();
    others().filter(|p| p["tab_id"] == tab).find_map(cwd_of)
}

/// Sidebar width in columns: `HERDR_PR_PANE_WIDTH`, else `ui.sidebar_width`
/// from herdr's config.toml, else herdr's default of 26 -- doubled, since the
/// PR pane needs more room than a sidebar.
pub fn target_width() -> i64 {
    if let Some(w) = std::env::var("HERDR_PR_PANE_WIDTH").ok().and_then(|v| v.parse().ok()) {
        return w;
    }
    let path = std::env::var("HERDR_CONFIG_PATH")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config/herdr/config.toml")));
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| {
            t.lines()
                .filter_map(|l| l.split_once('='))
                .find(|(k, _)| k.trim() == "sidebar_width")
                .and_then(|(_, v)| v.trim().parse().ok())
        })
        .unwrap_or(26)
        * 2
}

/// Resize our own split so the pane is `width` columns wide. `pane resize`
/// takes a ratio delta of the enclosing split, so converge in a few steps.
pub fn fit_width(width: i64) {
    let own = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    for attempt in 0..6 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let Ok(out) = Command::new(bin()).args(["pane", "layout", "--pane", &own]).output() else { return };
        let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else { return };
        let layout = &v["result"]["layout"];
        let Some(me) = layout["panes"].as_array().and_then(|ps| ps.iter().find(|p| p["pane_id"] == own.as_str())) else {
            return;
        };
        let (x, w) = (me["rect"]["x"].as_i64().unwrap_or(0), me["rect"]["width"].as_i64().unwrap_or(0));
        // Tightest horizontal split that contains us.
        let split = layout["splits"].as_array().and_then(|ss| {
            ss.iter()
                .filter(|s| s["direction"] == "right")
                .filter(|s| {
                    let sx = s["rect"]["x"].as_i64().unwrap_or(0);
                    let sw = s["rect"]["width"].as_i64().unwrap_or(0);
                    sx <= x && x + w <= sx + sw
                })
                .min_by_key(|s| s["rect"]["width"].as_i64().unwrap_or(i64::MAX))
        });
        let Some(split) = split else { return };
        let total = split["rect"]["width"].as_i64().unwrap_or(0).max(1);
        if (w - width).abs() <= 1 {
            return;
        }
        // We are the right-hand child (opened with --direction right): moving
        // our left edge right shrinks us.
        let (dir, amount) = if w > width { ("right", w - width) } else { ("left", width - w) };
        let _ = Command::new(bin())
            .args(["pane", "resize", "--pane", &own, "--direction", dir, "--amount"])
            .arg(format!("{:.4}", amount as f64 / total as f64))
            .output();
    }
}

/// Close the PR pane in the current tab if one is open, else open one beside
/// the focused pane. Runs as a plugin action, so context ids are in the env.
pub fn toggle() {
    let tab = std::env::var("HERDR_TAB_ID").unwrap_or_default();
    let target = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let existing = Command::new(bin())
        .args(["pane", "list"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .and_then(|v| {
            v["result"]["panes"].as_array().and_then(|ps| {
                ps.iter()
                    .find(|p| p["label"] == "PR" && p["tab_id"] == tab.as_str())
                    .and_then(|p| p["pane_id"].as_str().map(str::to_owned))
            })
        });
    match existing {
        Some(id) => {
            let _ = Command::new(bin()).args(["pane", "close", &id]).output();
        }
        None => {
            let target = rightmost_pane(&target).unwrap_or(target);
            let mut cmd = Command::new(bin());
            cmd.args(["plugin", "pane", "open", "--plugin", "herdr-pr-pane", "--entrypoint", "pr", "--placement", "split", "--direction", "right", "--no-focus"]);
            if !target.is_empty() {
                cmd.args(["--target-pane", &target]);
            }
            let _ = cmd.output();
        }
    }
}

/// Pane touching the tab's right edge (tallest if several), so splitting it
/// right yields a side pane at the far right instead of mid-layout.
fn rightmost_pane(from: &str) -> Option<String> {
    if from.is_empty() {
        return None;
    }
    let out = Command::new(bin()).args(["pane", "layout", "--pane", from]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let panes = v["result"]["layout"]["panes"].as_array()?;
    let n = |p: &Value, k: &str| p["rect"][k].as_i64().unwrap_or(0);
    panes
        .iter()
        .max_by_key(|p| (n(p, "x") + n(p, "width"), n(p, "height")))
        .and_then(|p| p["pane_id"].as_str().map(str::to_owned))
}
