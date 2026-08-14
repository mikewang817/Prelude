//! The unit of everything the launcher shows.

use serde::{Deserialize, Serialize};

/// What kind of thing a row is. Drives priority, colour, and which verbs the
/// action panel offers.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A question an agent has put to you and is blocked on.
    Msg,
    Agent,
    /// An agent process alive right now, as opposed to a session file.
    Run,
    Session,
    Config,
    /// One of Prelude's own preferences, carrying its current value.
    Setting,
    /// A search provider or scoped search waiting for its argument.
    Search,
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
            // Above even the agents. A run that has gone quiet *might* want
            // something; one of these has said so and is blocked until you
            // answer.
            //
            // Ten points is enough, because these numbers are compared to
            // each other and nothing else — `cache::by_rank` settles the
            // band before it ever looks at a score. It was not always so:
            // when everything went into one total, this had to sit a hundred
            // clear of Agent to survive a frecency bonus that reached sixty,
            // and the rest of the cluster, spanning twenty-five points, did
            // not survive it at all.
            Msg => 1010,
            // Agents occupy their own band, far enough above everything
            // else that learned ranking cannot lift another kind past them.
            // They are what this launcher is for; everything else is one
            // keystroke away through search.
            Agent => 1000,
            // Above skills and past sessions: something running and stuck
            // is more urgent than anything you might start.
            Run => 995,
            Skill => 990,
            Session => 980,
            Config => 975,
            // Below the agent cluster and above the search commands: these
            // are never what you opened the launcher for, and `set:` is the
            // only place they appear — `root_items` does not admit the kind,
            // so the band decides nothing but their order among themselves.
            Setting => 440,
            Search => 430,
            Translate => 420,
            Calc => 400,
            Port => 150,
            Proc => 148,
            Link => 147,
            Find => 146,
            Container => 145,
            Snippet => 140,
            Clip => 135,
            Ssh => 120,
            // What this project can do beats a static system command or an
            // app you could have launched from anywhere.
            Script => 100,
            App => 88,
            Sys => 86,
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
            Msg => (RED, "asking you"),
            Agent => (GREEN, "agent"),
            Run => (GREEN, "running"),
            Session => (MAGENTA, "session"),
            Config => (YELLOW, "config"),
            Setting => (YELLOW, "setting"),
            Search => (CYAN, "search"),
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

    /// The band a Quicklink sits in, whatever it points at.
    ///
    /// A quicklink is not a file that happens to be indexed; it is a name the
    /// person chose for something they expect to reach by typing that name.
    /// Left in the target's own band it was hopeless — `File` is 60 and
    /// `Link` 147, so a keyword the person invented sat below every scope
    /// command in `root_items`, and only an exactly-complete key ever came
    /// first, because `is_special` short-circuits before ranking happens at
    /// all. Typing three letters of a six-letter keyword put it back at the
    /// bottom of the list.
    ///
    /// Above `Setting` and `Search` so it beats the scope commands, and well
    /// below the agent cluster, which is what the launcher is for.
    pub const QUICKLINK: i64 = 460;

    /// A newer release. Below a question an agent is blocked on, because
    /// nothing outranks that, and above everything else.
    ///
    /// It was first put below the agent cluster on the reasoning that the
    /// launcher is not what you opened the launcher for — which is true, and
    /// produced a notice on line 31, under ten sessions. A notification you
    /// have to scroll to is not one. This row exists only while there is
    /// something to do about it, appears once per release, and leaves when the
    /// update is taken; on that basis it can lead.
    pub const UPDATE: i64 = 1005;

    /// The label shown for a Quicklink row, regardless of its target's kind.
    pub const QUICKLINK_STYLE: (&'static str, &'static str) =
        (crate::ansi::BRIGHT_CYAN, "quicklink");

    pub fn all() -> &'static [Kind] {
        use Kind::*;
        &[
            Msg, Agent, Run, Session, Config, Setting, Search, Translate, Calc, Port, Proc, Link, Find, Container, Snippet,
            Clip, Skill, Ssh, App, Sys, Script, History, Dir, Git, File, Path,
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
    /// Kind-specific payload; drives the action panel and Quick Look.
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
    /// Where this sits *within* its kind, according to the source.
    ///
    /// A source knows things the launcher cannot: which skill you actually
    /// invoke, which session is the newest, which run is the stuck one. That
    /// belonged in the ordering and had nowhere to go, so it was either
    /// thrown away — a skill used eight times sorted below four you had
    /// never touched, purely on the first letter — or smuggled in as the
    /// order rows happened to be generated in, which the first item to gain
    /// a frecency bonus then broke.
    ///
    /// Recorded *and* applied: `read_cached` rebuilds the score from kind
    /// plus this on the way back off disk, and setting it here means sources
    /// that never go through the cache behave identically.
    ///
    /// It cannot reach outside the kind — `cache::by_rank` settles the band
    /// first — so the number is free to be on whatever scale suits the
    /// source.
    pub fn rank(mut self, r: f64) -> Self {
        self.data.insert("rank".to_string(), format!("{r:.3}"));
        self.score = self.band() as f64 + r;
        self
    }

    /// Mark this row as a saved Quicklink and move it into the Quicklink band.
    ///
    /// `shape` is `fixed` or `template`. The *result* of a template — the Link
    /// row `g rust async` produces — is deliberately not one of these: it
    /// carries the key so its provider can be edited, but it is a search
    /// result, not a thing the person saved, and it must keep saying `open`.
    ///
    /// The score is reset rather than added to, so every quicklink starts
    /// level and frecency alone decides their order among themselves. Left at
    /// the target's priority they sorted Link before App before Dir before
    /// File — an order nobody chose and nothing on the row explained.
    pub fn quicklink(mut self, key: &str, shape: &str) -> Self {
        self.data.insert("ql".to_string(), shape.to_string());
        self.data.insert("quicklink".to_string(), key.to_string());
        self.score = Kind::QUICKLINK as f64;
        self
    }

    /// Is this a Quicklink the person saved, as opposed to a row that merely
    /// came out of one?
    pub fn is_quicklink(&self) -> bool {
        matches!(self.get("ql"), "fixed" | "template")
    }

    /// The band this row sorts in. Normally the kind's, but a Quicklink is
    /// banded by the fact that somebody named it — both shapes of one, since
    /// a template is every bit as much a keyword the person chose.
    pub fn band(&self) -> i64 {
        if self.get("update") == "available" {
            return Kind::UPDATE;
        }
        if self.is_quicklink() { Kind::QUICKLINK } else { self.kind.priority() }
    }

    /// Colour and label for the list.
    ///
    /// This column answers *what kind of thing is this row*, not what Enter
    /// will do to it. Almost every label is a noun naming a source — `agent`,
    /// `session`, `skill`, `mcp`, `clipboard`, `history`, `app`, `folder`,
    /// `branch`, `script` — and what Enter does is already stated twice
    /// elsewhere, in the footer and at the top of the `^K` panel.
    ///
    /// So both shapes of a Quicklink say `quicklink`, and `search` is left to
    /// mean the thing it actually names: a scope command into Prelude's own
    /// index. `Kind::Search` carries both populations, and they are not the
    /// same thing — `f:` is built in and goes to the file index, `gh <query>`
    /// is a line in the person's `quicklinks.toml` and goes to the web. That
    /// they behave alike on Enter is a coincidence of both needing an
    /// argument, and it is not what the column was being asked.
    pub fn style(&self) -> (&'static str, &'static str) {
        if self.get("update") == "available" {
            return (crate::ansi::YELLOW, "update");
        }
        if self.get("index_kind") == "folder" {
            return (crate::ansi::BLUE, "folder");
        }
        if self.is_quicklink() { Kind::QUICKLINK_STYLE } else { self.kind.style() }
    }

    pub fn put(mut self, k: &str, v: impl Into<String>) -> Self {
        self.data.insert(k.to_string(), v.into());
        self
    }
    pub fn get(&self, k: &str) -> &str {
        self.data.get(k).map(String::as_str).unwrap_or("")
    }
}
