//! Diagnose the setup, and measure the one thing that cannot be guessed.

use crate::ansi::*;
use crate::exec::which;
use crate::item::Kind;
use crate::paths;

pub fn run() -> i32 {
    let mut ok = true;
    let mut check = |label: String, good: bool, detail: &str| {
        let mark = if good { format!("{GREEN}✓{RESET}") } else { format!("{YELLOW}✗{RESET}") };
        let d = if detail.is_empty() { String::new() } else { format!("  {DIM}{detail}{RESET}") };
        println!("  {mark} {label}{d}");
        if !good {
            ok = false;
        }
    };

    println!("\n{CYAN}Prelude doctor{RESET}\n");
    let fzf = which("fzf");
    check("fzf installed".into(), fzf.is_some(),
          &fzf.map(|p| p.to_string_lossy().into_owned()).unwrap_or("brew install fzf".into()));
    check("zoxide (folder ranking)".into(), which("zoxide").is_some(), "optional");
    check("tmux (popup rendering)".into(), which("tmux").is_some(), "optional");

    let hist = std::env::var("HISTFILE").map(std::path::PathBuf::from)
        .unwrap_or_else(|_| paths::home().join(".zsh_history"));
    check("shell history readable".into(), hist.exists(), &hist.to_string_lossy());

    let t = std::time::Instant::now();
    let items = crate::cache::gather();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    check(format!("gathered {} candidates", items.len()), !items.is_empty(), &format!("{ms:.0}ms"));
    check("gather under 40ms budget".into(), ms < 40.0, &format!("{ms:.0}ms"));

    // The ambiguous-width probe: measured, never inferred.
    match crate::probe::ambiguous_width() {
        Some(w) => {
            let _ = crate::cache::write_atomic(&paths::cache().join("ambiguous_width"),
                                               w.to_string().as_bytes());
            check(format!("ambiguous-width probe: · = {w} column{}", if w > 1 { "s" } else { "" }),
                  true, "measured from your terminal");
        }
        None => check("ambiguous-width probe".into(), false, "needs a real terminal; assuming 1"),
    }

    let mut counts = std::collections::BTreeMap::new();
    for i in &items {
        *counts.entry(i.kind.style().1).or_insert(0usize) += 1;
    }
    println!("\n  {DIM}by source:{RESET} {}",
             counts.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  "));

    println!("\n  {CYAN}computed rows{RESET} {DIM}(type these, no search needed){RESET}");
    let u = which("units").is_some();
    println!("    {} units    {DIM}10kg to lb · 1gb to mb · 100 degF to degC{RESET}",
             if u { format!("{GREEN}✓{RESET}") } else { format!("{YELLOW}✗{RESET}") });
    println!("    {GREEN}✓{RESET} time     {DIM}now + 3 days · 1699999999{RESET}");
    println!("    {GREEN}✓{RESET} web      {DIM}github.com · localhost:3000 · https://example.com{RESET}");
    let ql = crate::compute::quicklinks();
    println!("    {GREEN}✓{RESET} links    {DIM}{}{RESET}",
             ql.keys().filter(|k| !k.is_empty()).cloned().collect::<Vec<_>>().join(" "));
    match std::fs::read_to_string(crate::compute::fileindex_path()) {
        Ok(t) => println!("    {GREEN}✓{RESET} f:name   {DIM}{} files indexed · prelude index to refresh{RESET}",
                          t.lines().count()),
        Err(_) => println!("    {YELLOW}✗{RESET} f:name   {DIM}no index — run:  prelude index{RESET}"),
    }

    println!("\n  {CYAN}translation{RESET} {DIM}(Apple, on-device){RESET}");
    if !crate::compute::translate_app().exists() {
        println!("    {YELLOW}✗{RESET} not built — run:  {DIM}prelude build-translate{RESET}");
    } else {
        match crate::compute::translate("这是一个测试", "en") {
            Ok(v) => println!("    {GREEN}✓{RESET} working  {DIM}这是一个测试 → {v}{RESET}"),
            Err(e) => println!("    {YELLOW}✗{RESET} {e}"),
        }
    }

    let skills = items.iter().filter(|i| i.kind == Kind::Skill).count();
    let mcps: Vec<&str> = items.iter().filter(|i| i.kind == Kind::Mcp).map(|i| i.title.as_str()).collect();
    println!("\n  {CYAN}agents{RESET}");
    println!("    {DIM}skills:{RESET} {skills} unique");
    println!("    {DIM}mcp servers:{RESET} {}", if mcps.is_empty() { "none".into() } else { mcps.join("  ") });
    let clis: Vec<&str> = ["claude", "codex", "pi", "cursor-agent", "opencode", "gemini"]
        .iter().copied().filter(|c| which(c).is_some()).collect();
    println!("    {DIM}CLIs on PATH:{RESET} {}", clis.join("  "));

    println!("\n  {CYAN}clipboard{RESET}");
    println!("    {DIM}watcher:{RESET} {}", if crate::clipd::is_running() {
        format!("{GREEN}running{RESET}") } else { format!("{YELLOW}not running{RESET} (starts on first search)") });
    println!();
    if ok { 0 } else { 1 }
}
