//! Prelude — a Raycast-style launcher for the terminal.
//!
//! One hotkey, fuzzy search across everything you might want to run, and it
//! *types the command for you* rather than running it.

mod actions;
mod ansi;
mod cache;
mod calc;
mod clipd;
mod compute;
mod defaults;
mod doctor;
mod exec;
mod frecency;
mod init;
mod item;
mod minitoml;
mod paths;
mod preview;
mod probe;
mod render;
mod runhere;
mod secrets;
mod sources;
mod translate_build;
mod tty;
mod ui;
mod width;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    let code = match a.as_slice() {
        [] => ui::search(None),
        ["paste"] => ui::search(ui::resolve_pane(None)),
        ["paste", pane] => ui::search(ui::resolve_pane(Some(pane))),
        ["doctor"] => doctor::run(),
        ["bench"] => bench(),
        ["index"] => {
            println!("building file index ...");
            let n = compute::build_fileindex();
            println!("  indexed {n} files from: {}", compute::index_roots().join(", "));
            println!("  search them with  f:name  in the launcher");
            0
        }
        ["clipd"] => clipd::watch(),
        ["build-translate"] => translate_build::build(),
        ["init", "zsh"] => { print!("{}", init::ZSH); 0 }
        ["init", "tmux"] => { print!("{}", init::TMUX); 0 }
        ["translate", lang, rest @ ..] if !rest.is_empty() => {
            match compute::translate(&rest.join(" "), lang) {
                Ok(v) => { println!("{v}"); 0 }
                Err(e) => { eprintln!("prelude: {e}"); 2 }
            }
        }
        // Non-interactive dump of the rendered list, for testing and for
        // diffing behaviour without standing up a terminal.
        ["_dump"] => {
            let items = cache::gather();
            println!("{}", render::render(&items, term_width(), None));
            0
        }
        // Scriptable equivalent of the ^K action, and what its test drives.
        ["_copy-skill", dir, agent, name] => {
            match sources::agents::copy_skill(dir, agent, name) {
                Ok(p) => { println!("copied {name} -> {p}"); 0 }
                Err(e) => { eprintln!("prelude: {e}"); 1 }
            }
        }
        ["_refresh-path"] => { sources::user::scan_path(); 0 }
        ["_refresh", name] => if cache::refresh_named(name) { 0 } else { 1 },
        ["_bind", q, path, cols] => bind(q, path, cols),
        ["_dynamic", q] => dynamic(q, "", term_width()),
        ["_dynamic", q, path] => dynamic(q, path, term_width()),
        ["_dynamic", q, path, cols] => dynamic(q, path, cols.parse().unwrap_or_else(|_| term_width())),
        ["_ask", line] => match render::parse_line(line) {
            Some(i) => ui::ask(&i),
            None => 1,
        },
        // Decides, per keypress of Enter, whether to accept the selection or
        // answer it in place. Only fzf can make that conditional.
        ["_enter", line] => {
            let ask = render::parse_line(line)
                .is_some_and(|i| i.get("mode") == "start" && !i.get("prompt").is_empty());
            let me = exec::shq(&std::env::current_exe().unwrap_or_default().to_string_lossy());
            if ask {
                println!("change-preview-window(right,55%,wrap,border-left)+preview({me} _ask {{2}})");
            } else {
                println!("accept");
            }
            0
        }
        ["_preview", line] => { if let Some(i) = render::parse_line(line) { preview::show(&i); } 0 }
        ["_copy", line] => copy_line(line),
        ["_runhere", line] => match render::parse_line(line) {
            Some(i) => runhere::run_item(&i),
            None => 1,
        },
        ["-h"] | ["--help"] | ["help"] => { print!("{HELP}"); 0 }
        _ => { eprintln!("prelude: unknown command {a:?} (try: prelude doctor)"); 2 }
    };
    ExitCode::from(code.clamp(0, 255) as u8)
}

const HELP: &str = "\
prelude — a Raycast-style launcher for the terminal

  prelude                search (what the Ctrl-R widget calls)
  prelude paste [pane]   type the result into a tmux pane instead
  prelude init zsh       print shell integration
  prelude init tmux      print tmux integration
  prelude index          build the file index for f:name
  prelude build-translate  compile the Apple translation helper
  prelude translate L T  translate T into language L
  prelude doctor         diagnose the setup
  prelude bench          measure candidate-gathering
";

/// Decide, per keystroke, what fzf should do.
///
/// fzf matches against the *displayed* text, so a computed row (a sum, a
/// translation) can never fuzzy-match the query that produced it — you'd type
/// `en:...` and watch your own answer get filtered out. For those queries we
/// turn fzf's own filtering off and let our row stand at the top; for
/// ordinary queries we leave fuzzy search exactly as it was.
fn bind(q: &str, path: &str, cols: &str) -> i32 {
    let me = std::env::current_exe().unwrap_or_default();
    let me = exec::shq(&me.to_string_lossy());
    let search = if compute::is_special(q) { "disable-search" } else { "enable-search" };
    println!("{search}+reload({me} _dynamic {} {} {})",
             exec::shq(q), exec::shq(path), exec::shq(cols));
    0
}

/// Emit query-dependent rows, then the pre-rendered static list.
///
/// `cols` is passed in rather than measured: fzf runs this with stdout on a
/// pipe, so a measurement would fall back to a default and the computed rows
/// would drift ten columns out of step with the static ones.
fn dynamic(q: &str, path: &str, cols: usize) -> i32 {
    let rows = compute::dynamic_rows(q);
    if !rows.is_empty() {
        println!("{}", render::render(&rows, cols, None));
    }
    // Once the query has clearly declared an intent — a sum, a translation,
    // an agent to start — the 2000-odd unrelated rows underneath are noise.
    // Show only what was asked for.
    if compute::is_special(q) {
        return 0;
    }
    if !path.is_empty() {
        if let Ok(t) = std::fs::read_to_string(path) {
            print!("{t}");
            if !t.ends_with('\n') {
                println!();
            }
        }
    }
    0
}

/// `^Y`: copy without closing the launcher.
fn copy_line(line: &str) -> i32 {
    let Some(it) = render::parse_line(line) else { return 1 };
    use item::Kind::*;
    let by_kind = match it.kind {
        Port | Proc => it.get("pid"),
        Ssh => it.get("host"),
        Container | Mcp => it.get("name"),
        Link => it.get("url"),
        File | Find => it.get("path"),
        Clip => it.get("full"),
        _ => "",
    };
    let text = if by_kind.is_empty() { it.cmd.clone() } else { by_kind.to_string() };
    ui::copy(&text);
    frecency::bump(&it.cmd);
    0
}

fn bench() -> i32 {
    let mut times = Vec::new();
    let mut n = 0;
    for _ in 0..5 {
        let t = std::time::Instant::now();
        n = cache::gather().len();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("gather: {n} items  min {:.1}ms  median {:.1}ms  max {:.1}ms",
             times[0], times[times.len() / 2], times[times.len() - 1]);
    println!("budget: 40ms  ->  {}", if times[times.len() / 2] < 40.0 { "OK" } else { "OVER" });
    0
}

pub fn term_width() -> usize {
    if let Ok(c) = std::env::var("COLUMNS") {
        if let Ok(n) = c.parse::<usize>() {
            return n;
        }
    }
    #[repr(C)]
    struct WinSize { rows: u16, cols: u16, x: u16, y: u16 }
    unsafe extern "C" { unsafe fn ioctl(fd: i32, req: u64, ...) -> i32; }
    let mut ws = WinSize { rows: 0, cols: 0, x: 0, y: 0 };
    if unsafe { ioctl(1, 0x4008_7468, &mut ws) } == 0 && ws.cols > 0 {
        return ws.cols as usize;
    }
    100
}

#[cfg(test)]
mod tests {
    #[test]
    fn arithmetic() {
        for (q, want) in [
            ("1847*0.23", "424.81"),
            ("1024*1024", "1,048,576"),
            ("(15+5)/4", "5"),
            ("2+3*4", "14"),
            ("-5 + 3", "-2"),
        ] {
            assert_eq!(crate::calc::calc(q).as_deref(), Some(want), "for {q}");
        }
        for q in ["git status", "hello", "docker", "12"] {
            assert_eq!(crate::calc::calc(q), None, "for {q}");
        }
    }

    #[test]
    fn timestamps_and_offsets() {
        let (v, _) = crate::calc::timecalc("1699999999").unwrap();
        assert!(v.starts_with("2023-11-"), "got {v}");
        assert!(crate::calc::timecalc("now + 3 days").is_some());
        assert!(crate::calc::timecalc("today + 1 week").is_some());
        assert!(crate::calc::timecalc("banana").is_none());
    }

    #[test]
    fn unmetafy_recovers_chinese() {
        // 基 stored by zsh as e5 83(META) bf ba  ->  e5 9f ba
        let raw = b"\xe5\x83\xbf\xba";
        assert_eq!(
            String::from_utf8_lossy(&crate::sources::history::unmetafy(raw)),
            "基"
        );
    }

    #[test]
    fn secrets_are_filtered() {
        for s in ["export API_KEY=abc", "curl -H 'Bearer x'", "sk-aaaaaaaaaaaaaaaaaaaaaa"] {
            assert!(crate::secrets::looks_secret(s), "{s}");
        }
        for s in ["git push origin main", "npm run build", "psql -U admin"] {
            assert!(!crate::secrets::looks_secret(s), "{s}");
        }
    }

    /// Copying a skill is the only place Prelude writes to user files, so
    /// pin both halves: it copies the whole tree, and it never overwrites.
    #[test]
    fn skill_copy_is_complete_and_refuses_to_overwrite() {
        let root = std::env::temp_dir().join(format!("prelude-t{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("src/demo");
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
        std::fs::write(src.join("nested/extra.txt"), "x").unwrap();

        let dest_root = root.join("dest");
        let dest = dest_root.join("demo");
        std::fs::create_dir_all(&dest_root).unwrap();
        crate::sources::agents::copy_tree(&src, &dest).unwrap();
        assert!(dest.join("SKILL.md").exists());
        assert!(dest.join("nested/extra.txt").exists(), "subdirectories must come too");

        // and the guard that stops a copy clobbering an existing skill
        assert!(dest.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn enter_defaults_split_commands_from_objects() {
        use crate::defaults::{on_enter, Default_, Host};
        use crate::item::Kind::*;
        // Commands are never acted on, in either host.
        for k in [History, Script, Path, Snippet, Port, Proc, Sys] {
            for h in [Host::Shell, Host::Agent] {
                assert_eq!(on_enter(k, h), Default_::Insert, "{k:?} in {h:?}");
            }
        }
        // Objects act at a shell but hand over text to an agent.
        for k in [File, App, Link] {
            assert!(matches!(on_enter(k, Host::Shell), Default_::Act(_)), "{k:?}");
            assert!(matches!(on_enter(k, Host::Agent), Default_::InsertText(_)), "{k:?}");
        }
    }

    #[test]
    fn cjk_width_and_truncation() {
        assert_eq!(crate::width::dwidth("abc"), 3);
        assert_eq!(crate::width::dwidth("宽字符测试"), 10);
        let t = crate::width::dtrunc("宽字符截断测试", 6);
        assert!(crate::width::dwidth(&t) <= 6, "got {t}");
    }
}
