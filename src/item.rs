//! The unit of everything the launcher shows.

use serde::{Deserialize, Serialize};

/// What kind of thing a row is. Drives priority, colour, and which verbs the
/// action panel offers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Translate,
    Calc,
    Port,
    Proc,
    Link,
    Find,
    Container,
    Snippet,
    Clip,
    Skill,
    Mcp,
    Ssh,
    App,
    Sys,
    Script,
    History,
    Dir,
    Git,
    File,
    Path,
}

impl Kind {
    /// Breaks ties when fuzzy scores are equal. Without this the 1900 $PATH
    /// binaries outrank a project's own scripts.
    pub fn priority(self) -> i64 {
        use Kind::*;
        match self {
            Translate => 420,
            Calc => 400,
            Port => 150,
            Proc => 148,
            Link => 147,
            Find => 146,
            Container => 145,
            Snippet => 140,
            Clip => 135,
            Skill => 130,
            Mcp => 125,
            Ssh => 120,
            App => 112,
            Sys => 110,
            Script => 100,
            History => 90,
            Dir => 80,
            Git => 70,
            File => 60,
            Path => 40,
        }
    }

    /// (ansi colour, label shown in the list)
    pub fn style(self) -> (&'static str, &'static str) {
        use crate::ansi::*;
        use Kind::*;
        match self {
            Translate => (CYAN, "translate"),
            Calc => (YELLOW, "="),
            Port => (RED, "port"),
            Proc => (RED, "process"),
            Link => (CYAN, "open"),
            Find => (DIM, "file"),
            Container => (BLUE, "container"),
            Snippet => (YELLOW, "snippet"),
            Clip => (CYAN, "clipboard"),
            Skill => (MAGENTA, "skill"),
            Mcp => (MAGENTA, "mcp"),
            Ssh => (CYAN, "ssh"),
            App => (BLUE, "app"),
            Sys => (YELLOW, "system"),
            Script => (GREEN, "script"),
            History => (DIM, "history"),
            Dir => (BLUE, "folder"),
            Git => (MAGENTA, "branch"),
            File => (DIM, "file"),
            Path => (DIM, "command"),
        }
    }

    pub fn all() -> &'static [Kind] {
        use Kind::*;
        &[
            Translate, Calc, Port, Proc, Link, Find, Container, Snippet, Clip,
            Skill, Mcp, Ssh, App, Sys, Script, History, Dir, Git, File, Path,
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    /// What lands on your prompt.
    pub cmd: String,
    /// What you see in the list. Defaults to `cmd`.
    pub title: String,
    pub kind: Kind,
    /// Free-text right-hand side, used when `fields` is empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    /// Structured right-hand side. Given these, the renderer lays out real
    /// per-kind columns instead of joining loose text with separators —
    /// which is what made pid and memory jitter between rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Kind-specific payload; drives the action panel and the detail pane.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub data: Map,
    #[serde(skip)]
    pub score: f64,
}

pub type Map = std::collections::BTreeMap<String, String>;

impl Item {
    pub fn new(cmd: impl Into<String>, kind: Kind) -> Self {
        let cmd = cmd.into();
        Item {
            title: cmd.clone(),
            cmd,
            kind,
            subtitle: String::new(),
            fields: Vec::new(),
            cwd: None,
            data: Map::new(),
            score: kind.priority() as f64,
        }
    }
    pub fn title(mut self, t: impl Into<String>) -> Self {
        self.title = t.into();
        self
    }
    pub fn sub(mut self, s: impl Into<String>) -> Self {
        self.subtitle = s.into();
        self
    }
    pub fn fields<I: Into<String>>(mut self, f: impl IntoIterator<Item = I>) -> Self {
        self.fields = f.into_iter().map(Into::into).collect();
        self
    }
    pub fn cwd(mut self, c: impl Into<String>) -> Self {
        self.cwd = Some(c.into());
        self
    }
    pub fn put(mut self, k: &str, v: impl Into<String>) -> Self {
        self.data.insert(k.to_string(), v.into());
        self
    }
    pub fn get(&self, k: &str) -> &str {
        self.data.get(k).map(String::as_str).unwrap_or("")
    }
}
