//! Diagnose the setup, and measure the one thing that cannot be guessed.

use crate::ansi::*;
use crate::exec::which;
use crate::item::Kind;
use crate::paths;

pub fn mcp() -> i32 {
    // An explicit diagnostic may pay for authoritative owner CLI health.
    let _ = crate::cache::refresh_named("mcp");
    let servers = crate::cache::read_cached("mcp");
    let now = crate::frecency::now() as u64;
    let mut ok = true;
    let mut seen = std::collections::BTreeSet::new();
    println!("\n{CYAN}Prelude doctor · MCP{RESET}\n");
    if servers.is_empty() {
        println!("  {YELLOW}✗{RESET} no MCP servers reported by installed Agent CLIs\n");
        return 1;
    }
    for server in &servers {
        let owner = server.get("agent");
        let name = server.get("name");
        let duplicate = !seen.insert((owner.to_string(), name.to_lowercase()));
        let health_at = server.get("health_checked_at").parse::<u64>().unwrap_or(0);
        let tools_at = server.get("tools_checked_at").parse::<u64>().unwrap_or(0);
        let health_stale = health_at == 0 || now.saturating_sub(health_at) > 120;
        let tools_status = server.get("tools_status");
        let tools_stale = !matches!(tools_status, "unsupported" | "disabled")
            && (tools_at == 0 || now.saturating_sub(tools_at) > 600);
        let healthy = server.get("health") == "ok";
        let transport_known = matches!(server.get("transport"), "stdio" | "http" | "sse" | "hosted");
        let retained_definition = !server.get("def").is_empty();
        let mark = if healthy && !health_stale && !tools_stale && !duplicate
            && transport_known && !retained_definition {
            format!("{GREEN}✓{RESET}")
        } else {
            ok = false;
            format!("{YELLOW}✗{RESET}")
        };
        println!(
            "  {mark} {owner:<8} {name}  {DIM}{} · {} · tools {}{RESET}",
            server.get("transport"), server.get("health"), tools_status,
        );
        for issue in [
            duplicate.then_some("duplicate owner/name definition"),
            health_stale.then_some("health snapshot is stale or missing"),
            tools_stale.then_some("tool inventory is stale or missing"),
            (!transport_known).then_some("transport is unknown"),
            retained_definition.then_some("complete definition was retained — privacy violation"),
        ].into_iter().flatten() {
            println!("      {YELLOW}{issue}{RESET}");
        }
        if !healthy {
            println!("      {YELLOW}health requires attention: {}{RESET}", server.get("health"));
        }
        if server.get("sensitive") == "true" {
            println!("      {DIM}private definition fields are omitted from retained data{RESET}");
        }
        if server.get("portable") == "false" {
            println!("      {DIM}owner-account only; no transferable local definition{RESET}");
        }
    }
    println!();
    if ok { 0 } else { 1 }
}

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

    let skill_rows: Vec<&crate::item::Item> = items.iter().filter(|i| i.kind == Kind::Skill).collect();
    let skills = skill_rows.len();
    let divergent = skill_rows.iter().filter(|skill| skill.get("integrity") == "divergent").count();
    let unknown = skill_rows.iter().filter(|skill| skill.get("integrity") == "unknown").count();
    let sensitive = skill_rows.iter().flat_map(|skill| crate::capability::copies(skill))
        .filter(|copy| copy.sensitive_files > 0).count();
    let mcp_rows: Vec<&crate::item::Item> = items.iter().filter(|i| i.kind == Kind::Mcp).collect();
    let mcps: Vec<&str> = mcp_rows.iter().map(|i| i.title.as_str()).collect();
    let unhealthy = mcp_rows.iter().filter(|server| server.get("health") != "ok").count();
    let private = mcp_rows.iter().filter(|server| server.get("sensitive") == "true").count();
    let tools_ok = mcp_rows.iter().filter(|server| server.get("tools_status") == "ok").count();
    let tools_failed = mcp_rows.iter().filter(|server| server.get("tools_status") == "failed").count();
    let tools_pending = mcp_rows.iter().filter(|server| server.get("tools_status") == "pending").count();
    println!("\n  {CYAN}agents{RESET}");
    println!("    {DIM}skills:{RESET} {skills} unique · {divergent} divergent · {unknown} unhashed");
    println!("    {DIM}skill privacy:{RESET} {sensitive} copies contain redacted credential-like lines");
    println!("    {DIM}mcp servers:{RESET} {}", if mcps.is_empty() { "none".into() } else { mcps.join("  ") });
    println!("    {DIM}mcp health:{RESET} {unhealthy} need attention · {private} definitions keep private fields out of cache");
    println!("    {DIM}mcp tools:{RESET} {tools_ok} inventoried · {tools_failed} failed · {tools_pending} pending");
    let clis: Vec<&str> = ["claude", "codex", "pi", "cursor-agent", "opencode", "gemini"]
        .iter().copied().filter(|c| which(c).is_some()).collect();
    println!("    {DIM}CLIs on PATH:{RESET} {}", clis.join("  "));

    println!("\n  {CYAN}clipboard{RESET}");
    println!("    {DIM}watcher:{RESET} {}", if crate::clipd::is_running() {
        format!("{GREEN}running{RESET}") } else { format!("{YELLOW}not running{RESET} (starts on first search)") });
    println!();
    if ok { 0 } else { 1 }
}
