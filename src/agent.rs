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
    resume: fn(&str) -> String,
    prompt: fn(&str) -> String,
    ask: fn(&str) -> Vec<String>,
    fork: Option<fn(&str) -> String>,
}

impl Spec {
    pub fn resume(&self, id: &str) -> String {
        (self.resume)(id)
    }

    pub fn prompt(&self, prompt: &str) -> String {
        (self.prompt)(prompt)
    }

    pub fn ask(&self, prompt: &str) -> Vec<String> {
        (self.ask)(prompt)
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
        resume: |id| format!("claude --resume {id}"),
        prompt: |prompt| format!("claude {}", shq(prompt)),
        ask: |prompt| vec!["claude".into(), "-p".into(), prompt.into()],
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
        resume: |id| format!("codex resume {id}"),
        prompt: |prompt| format!("codex {}", shq(prompt)),
        ask: |prompt| {
            vec![
                "codex".into(),
                "exec".into(),
                "--skip-git-repo-check".into(),
                prompt.into(),
            ]
        },
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
        resume: |id| format!("pi --session {id}"),
        prompt: |prompt| format!("pi {}", shq(prompt)),
        ask: |prompt| vec!["pi".into(), "--print".into(), prompt.into()],
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
        resume: |id| format!("opencode --session {id}"),
        prompt: |prompt| format!("opencode run {}", shq(prompt)),
        ask: |prompt| vec!["opencode".into(), "run".into(), prompt.into()],
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
        resume: |id| format!("kimi --session {id}"),
        // No interactive-with-prompt form: `kimi --help` lists `[command]`
        // rather than a positional `[prompt...]`, so the one shape that is
        // known to work carries the text as `--prompt` and prints. Same
        // arrangement as opencode, whose `run` is also both.
        prompt: |prompt| format!("kimi --prompt {}", shq(prompt)),
        ask: |prompt| vec!["kimi".into(), "--prompt".into(), prompt.into()],
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
        resume: |id| format!("cursor-agent --resume {id}"),
        prompt: |prompt| format!("cursor-agent {}", shq(prompt)),
        ask: |prompt| vec!["cursor-agent".into(), "-p".into(), prompt.into()],
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
            assert!(!spec.resume("session-id").is_empty());
            assert!(!spec.prompt("question").is_empty());
            assert!(!spec.ask("question").is_empty());
            assert_eq!(spec.fork("session-id").is_some(), spec.capabilities.fork);
            assert_eq!(spec.operation_labels().contains(&"resume"), spec.capabilities.resume);
            assert_eq!(spec.operation_labels().contains(&"fork"), spec.capabilities.fork);
            assert_eq!(spec.operation_labels().contains(&"one-off ask"), spec.capabilities.ask);
            // Every command a Spec can produce names the executable it is for,
            // so a row cannot advertise one Agent and hand over another's.
            assert!(spec.resume("id").starts_with(spec.name));
            assert!(spec.prompt("q").starts_with(spec.name));
            assert_eq!(spec.ask("q").first().map(String::as_str), Some(spec.name));
        }
    }
}
