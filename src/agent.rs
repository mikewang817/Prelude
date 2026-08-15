//! The built-in Agent provider registry.
//!
//! An Agent row is a launcher object, not a pile of `match "claude"`
//! statements. This registry owns the facts that every surface needs in order
//! to decide what it may honestly offer: invocation syntax, the settings file,
//! and one-run capability support. Native Session parsing, process discovery
//! and diagnostics remain in their own modules; they consume these facts
//! rather than maintaining another list of supported Agents.

use crate::exec::shq;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Capabilities {
    pub resume: bool,
    pub fork: bool,
    pub ask: bool,
    /// What the owner's CLI can report without claiming to describe an
    /// already-running process: `directory`, `subsystem`, or `none`.
    pub effective_config: &'static str,
}

pub struct Spec {
    pub name: &'static str,
    pub settings: &'static str,
    pub capabilities: Capabilities,
    /// `None` where the CLI has no way to resume a *named* conversation.
    ///
    /// Optional for the same reason `fork` is, and the reason is the rule this
    /// whole registry exists to keep: a command assembled from a syntax nobody
    /// verified looks right on the clipboard and fails after the launcher has
    /// closed. Several CLIs offer only `--continue` — the most recent session,
    /// no id — and for those a Session row has nothing honest to hand over.
    resume: Option<fn(&str) -> String>,
    prompt: fn(&str) -> String,
    /// `None` where no non-interactive form was found in the CLI's own help.
    ask: Option<fn(&str) -> Vec<String>>,
    fork: Option<fn(&str) -> String>,
}

impl Spec {
    pub fn resume(&self, id: &str) -> Option<String> {
        self.resume.map(|resume| resume(id))
    }

    pub fn prompt(&self, prompt: &str) -> String {
        (self.prompt)(prompt)
    }

    pub fn ask(&self, prompt: &str) -> Option<Vec<String>> {
        self.ask.map(|ask| ask(prompt))
    }

    pub fn fork(&self, id: &str) -> Option<String> {
        self.fork.map(|fork| fork(id))
    }

    pub fn executable(&self) -> Option<PathBuf> {
        crate::exec::which(self.name)
    }

    /// The conventional settings path, whether or not the file exists. Quick
    /// Look can therefore say where this Agent belongs without starting its
    /// CLI; actions still require `existing_settings` before offering Open.
    pub fn settings_path(&self) -> PathBuf {
        crate::paths::home().join(self.settings)
    }

    pub fn existing_settings(&self) -> Option<PathBuf> {
        self.settings_path().is_file().then(|| self.settings_path())
    }

    pub fn operation_labels(&self) -> Vec<&'static str> {
        let mut labels = vec!["start", "sessions"];
        if self.capabilities.ask {
            labels.push("one-off ask");
        }
        if self.capabilities.resume {
            labels.push("resume");
        }
        if self.capabilities.fork {
            labels.push("fork");
        }
        labels
    }
}

pub const SPECS: &[Spec] = &[
    Spec {
        name: "claude",
        settings: ".claude/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: true,
            ask: true,
            effective_config: "subsystem",
        },
        resume: Some(|id| format!("claude --resume {id}")),
        prompt: |prompt| format!("claude {}", shq(prompt)),
        ask: Some(|prompt| vec!["claude".into(), "-p".into(), prompt.into()]),
        fork: Some(|id| format!("claude --resume {} --fork-session", shq(id))),
    },
    Spec {
        name: "codex",
        settings: ".codex/config.toml",
        capabilities: Capabilities {
            resume: true,
            fork: true,
            ask: true,
            effective_config: "directory",
        },
        resume: Some(|id| format!("codex resume {id}")),
        prompt: |prompt| format!("codex {}", shq(prompt)),
        ask: Some(|prompt| {
            vec![
                "codex".into(),
                "exec".into(),
                "--skip-git-repo-check".into(),
                prompt.into(),
            ]
        }),
        fork: Some(|id| format!("codex fork {}", shq(id))),
    },
    Spec {
        name: "pi",
        settings: ".pi/agent/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: true,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("pi --session {id}")),
        prompt: |prompt| format!("pi {}", shq(prompt)),
        ask: Some(|prompt| vec!["pi".into(), "--print".into(), prompt.into()]),
        fork: Some(|id| format!("pi --fork {}", shq(id))),
    },
    Spec {
        name: "opencode",
        settings: ".config/opencode/opencode.jsonc",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "directory",
        },
        resume: Some(|id| format!("opencode --session {id}")),
        prompt: |prompt| format!("opencode run {}", shq(prompt)),
        ask: Some(|prompt| vec!["opencode".into(), "run".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "kimi",
        settings: ".kimi-code/config.toml",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("kimi --session {id}")),
        // No interactive-with-prompt form: `kimi --help` lists `[command]`
        // rather than a positional `[prompt...]`, so the one shape that is
        // known to work carries the text as `--prompt` and prints. Same
        // arrangement as opencode, whose `run` is also both.
        prompt: |prompt| format!("kimi --prompt {}", shq(prompt)),
        ask: Some(|prompt| vec!["kimi".into(), "--prompt".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "cursor-agent",
        settings: ".cursor/cli-config.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("cursor-agent --resume {id}")),
        prompt: |prompt| format!("cursor-agent {}", shq(prompt)),
        ask: Some(|prompt| vec!["cursor-agent".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "omp",
        settings: ".omp/agent/config.yml",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("omp --resume {id}")),
        prompt: |prompt| format!("omp {}", shq(prompt)),
        ask: Some(|prompt| vec!["omp".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "gemini",
        settings: ".gemini/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("gemini -r {}", shq(id))),
        prompt: |prompt| format!("gemini -i {}", shq(prompt)),
        ask: Some(|prompt| vec!["gemini".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "qwen",
        settings: ".qwen/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("qwen -r {}", shq(id))),
        prompt: |prompt| format!("qwen -i {}", shq(prompt)),
        ask: Some(|prompt| vec!["qwen".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "copilot",
        settings: ".copilot/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("copilot --resume={}", shq(id))),
        prompt: |prompt| format!("copilot -i {}", shq(prompt)),
        ask: Some(|prompt| vec!["copilot".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "qoder",
        settings: ".qoder/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: true,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("qoder -r {}", shq(id))),
        prompt: |prompt| format!("qoder {}", shq(prompt)),
        ask: Some(|prompt| vec!["qoder".into(), "-p".into(), prompt.into()]),
        fork: Some(|id| format!("qoder -r {} --fork-session", shq(id))),
    },
    Spec {
        name: "droid",
        settings: ".factory/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("droid -r {}", shq(id))),
        prompt: |prompt| format!("droid {}", shq(prompt)),
        ask: Some(|prompt| vec!["droid".into(), "exec".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "grok",
        settings: ".grok/config.toml",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: false,
            effective_config: "none",
        },
        // No non-interactive form found in its help: `--output-format` shapes
        // the output of a session it does not start.
        resume: Some(|id| format!("grok -r {}", shq(id))),
        prompt: |prompt| format!("grok {}", shq(prompt)),
        ask: None,
        fork: None,
    },
    Spec {
        name: "agy",
        settings: ".agy/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("agy --conversation {}", shq(id))),
        prompt: |prompt| format!("agy -i {}", shq(prompt)),
        ask: Some(|prompt| vec!["agy".into(), "-p".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "cline",
        settings: ".cline/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: false,
            effective_config: "none",
        },
        // `--json` changes the output format, not whether it is interactive.
        resume: Some(|id| format!("cline --id {}", shq(id))),
        prompt: |prompt| format!("cline {}", shq(prompt)),
        ask: None,
        fork: None,
    },
    Spec {
        name: "mastracode",
        settings: ".mastracode/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        // `--prompt` is required either way, so start and ask are one form.
        resume: Some(|id| format!("mastracode --thread {}", shq(id))),
        prompt: |prompt| format!("mastracode --prompt {}", shq(prompt)),
        ask: Some(|prompt| vec!["mastracode".into(), "--prompt".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "amp",
        settings: ".config/amp/settings.json",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: true,
            effective_config: "none",
        },
        resume: Some(|id| format!("amp threads continue {}", shq(id))),
        prompt: |prompt| format!("amp {}", shq(prompt)),
        ask: Some(|prompt| vec!["amp".into(), "-x".into(), prompt.into()]),
        fork: None,
    },
    Spec {
        name: "kiro-cli",
        settings: ".kiro/settings",
        capabilities: Capabilities {
            resume: true,
            fork: false,
            ask: false,
            effective_config: "none",
        },
        // Its help is a subcommand menu; no non-interactive chat form is shown.
        resume: Some(|id| format!("kiro-cli chat --resume-id {}", shq(id))),
        prompt: |prompt| format!("kiro-cli chat {}", shq(prompt)),
        ask: None,
        fork: None,
    },
    Spec {
        name: "kilo",
        settings: ".config/kilo/settings.json",
        capabilities: Capabilities {
            resume: false,
            fork: false,
            ask: false,
            effective_config: "none",
        },
        // Only `-c/--continue`, which is the most recent session and takes no
        // id, so a named conversation has nothing honest to hand over.
        resume: None,
        prompt: |prompt| format!("kilo {}", shq(prompt)),
        ask: None,
        fork: None,
    },
];

pub fn get(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|spec| spec.name == name)
}

pub fn installed() -> Vec<&'static str> {
    SPECS
        .iter()
        .filter(|spec| spec.executable().is_some())
        .map(|spec| spec.name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_capabilities_and_commands_are_one_consistent_answer() {
        for spec in SPECS {
            assert_eq!(spec.resume("session-id").is_some(), spec.capabilities.resume);
            assert_eq!(spec.ask("question").is_some(), spec.capabilities.ask);
            assert_eq!(spec.fork("session-id").is_some(), spec.capabilities.fork);
            assert!(!spec.prompt("question").is_empty());
            assert_eq!(spec.operation_labels().contains(&"resume"), spec.capabilities.resume);
            assert_eq!(spec.operation_labels().contains(&"fork"), spec.capabilities.fork);
            assert_eq!(spec.operation_labels().contains(&"one-off ask"), spec.capabilities.ask);
            // Every command a Spec can produce names the executable it is for,
            // so a row cannot advertise one Agent and hand over another's.
            let named = |command: &str| {
                command.split_whitespace().next() == Some(spec.name)
            };
            assert!(spec.resume("id").is_none_or(|c| named(&c)), "{}", spec.name);
            assert!(named(&spec.prompt("q")), "{}", spec.name);
            assert!(spec.fork("id").is_none_or(|c| named(&c)), "{}", spec.name);
            assert!(
                spec.ask("q").is_none_or(|a| a.first().map(String::as_str) == Some(spec.name)),
                "{}", spec.name
            );
        }
    }
}
