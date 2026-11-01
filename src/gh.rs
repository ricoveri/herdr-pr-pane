//! GitHub access through the `gh` CLI (uses the user's existing auth).

use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub details_url: Option<String>,
    #[serde(default)]
    pub target_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CheckState {
    Pass,
    Fail,
    Pending,
    Skipped,
}

impl Check {
    pub fn title(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.context.clone())
            .unwrap_or_else(|| "(unnamed)".into())
    }
    pub fn url(&self) -> Option<&str> {
        self.details_url
            .as_deref()
            .or(self.target_url.as_deref())
            .filter(|u| !u.is_empty())
    }
    pub fn result(&self) -> CheckState {
        // CheckRun: status + conclusion. StatusContext: state.
        let concl = self.conclusion.as_deref().unwrap_or("");
        let state = self.state.as_deref().unwrap_or("");
        match (self.status.as_deref(), concl, state) {
            (Some(s), _, _) if s != "COMPLETED" => CheckState::Pending,
            (_, "SUCCESS", _) | (_, _, "SUCCESS") => CheckState::Pass,
            (_, "SKIPPED" | "NEUTRAL", _) => CheckState::Skipped,
            (_, "", "PENDING" | "EXPECTED") => CheckState::Pending,
            (_, "", "") => CheckState::Pending,
            _ => CheckState::Fail,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub author: Author,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub url: String,
}

impl Comment {
    pub fn is_bot(&self) -> bool {
        let l = self.author.login.to_lowercase();
        l.ends_with("[bot]") || l.ends_with("bot") || l.ends_with("bbitai") || l == "github-actions"
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pr {
    pub number: u64,
    pub state: String,
    pub title: String,
    #[serde(default)]
    pub is_draft: bool,
    pub url: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub base_ref_name: String,
    #[serde(default)]
    pub status_check_rollup: Vec<Check>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub reviews: Vec<Comment>,
}

impl Pr {
    /// Issue comments and review bodies merged, newest first, empty bodies dropped.
    pub fn feed(&self) -> Vec<Comment> {
        let mut all: Vec<Comment> = self
            .comments
            .iter()
            .chain(self.reviews.iter())
            .filter(|c| !c.body.trim().is_empty())
            .cloned()
            .collect();
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all
    }
}

#[derive(Debug, Clone)]
pub enum Fetched {
    Pr(Box<Pr>),
    NoPr,
    NotARepo,
    Error(String),
}

pub fn branch_of(cwd: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", cwd, "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!b.is_empty() && b != "HEAD").then_some(b)
}

pub fn fetch(cwd: &str, branch: &str) -> Fetched {
    let out = Command::new("gh")
        .current_dir(cwd)
        .args([
            "pr",
            "view",
            branch,
            "--json",
            "number,state,title,isDraft,url,updatedAt,baseRefName,statusCheckRollup,comments,reviews",
        ])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Fetched::Error(format!("cannot run gh: {e}")),
    };
    if out.status.success() {
        return match serde_json::from_slice::<Pr>(&out.stdout) {
            Ok(pr) => Fetched::Pr(Box::new(pr)),
            Err(e) => Fetched::Error(format!("bad gh output: {e}")),
        };
    }
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    if err.contains("no pull requests found") {
        Fetched::NoPr
    } else if err.contains("not a git repository") {
        Fetched::NotARepo
    } else if err.contains("gh auth login") || err.contains("authentication") {
        Fetched::Error("gh not authenticated: run `gh auth login`".into())
    } else {
        Fetched::Error(err.lines().next().unwrap_or("gh failed").to_owned())
    }
}

pub fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let _ = Command::new(opener).arg(url).spawn();
}
