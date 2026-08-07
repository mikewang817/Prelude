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
            Mcp => 985,
            Session => 980,
            Config => 975,
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

    /// Is `cmd` something a shell could actually run?
    ///
    /// The panel used to append "Run in the shell" to every kind that had
    /// not already claimed it, which is not a fallback but a bug with a
    /// fallback's manners: a calculator row is `424.81`, a translation row
    /// is the translation, and a skill row is `/cnipa-ooa`, which a shell
    /// answers with "no such file". See `docs/ACTIONS.md`, R3.
    /// `App`, `Link` and `Dir` are deliberately absent. Their `cmd` reads
    /// like a command — `open -a Zed`, `open https://…`, `cd /tmp` — but the
    /// row denotes an application, a URL and a folder. Enter already launches
    /// the first two, so "Run in the shell below" was the same action under a
    /// worse name; and `cd` in a subshell that exits immediately does
    /// nothing at all.
    pub fn is_command_line(self) -> bool {
        use Kind::*;
        matches!(
            self,
            History | Script | Path | Snippet | Ssh | Container | Git | Port | Proc | Sys | Clip
                | Agent | Session | Mcp
        )
    }

    /// Is "ask an agent about this" a sentence that means anything?
    ///
    /// It hands over `about this: <the row>`, which is a real question for a
    /// command you did not write, a port you did not open, or a file you do
    /// not recognise. It is not one for a row that *is* an agent, a skill, a
    /// sum, or a translation: `about this: pi` asks claude about the word
    /// "pi", and the panel offered exactly that.
    pub fn worth_asking_about(self) -> bool {
        use Kind::*;
        matches!(
            self,
            History | Script | Path | Sys | Port | Proc | Container | File | Find | Config | Git
                | Ssh | Snippet | Clip
        )
    }

    /// Does running it take over the terminal?
    ///
    /// "Run here" renders output inside the launcher, which is right for a
    /// command that prints and wrong for one that paints: an agent, a
    /// resumed session, ssh and `docker exec -it` are all full-screen, and
    /// belong in a real terminal or not at all.
    pub fn is_interactive(self) -> bool {
        use Kind::*;
        matches!(self, Ssh | Agent | Session | Container)
    }

    pub fn all() -> &'static [Kind] {
        use Kind::*;
        &[
            Msg, Agent, Session, Config, Translate, Calc, Port, Proc, Link, Find, Container, Snippet,
            Clip, Skill, Mcp, Ssh, App, Sys, Script, History, Dir, Git, File, Path,
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
        self.score = self.kind.priority() as f64 + r;
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
