//! `prelude://` — a name the person chose, acted on from outside the launcher.
//!
//! The panel cannot be revealed from outside, so a URL that wanted to show a
//! filtered launcher would have to *build* one, which is the design `A press
//! reveals; it never creates` exists to have removed. A deeplink therefore
//! does the thing instead: `prelude://run?alias=browser` opens Chrome, with no
//! interface in between. Bound to a chord in whatever hotkey tool somebody
//! already has, that is a hotkey per command at no cost to this program.
//!
//! **A URL is untrusted input.** Any web page can navigate to `prelude://…`,
//! so the verb table here is a security boundary rather than a convenience:
//!
//! * The only thing a URL may name is something the person themselves created
//!   — an alias, which resolves to a stable object key and nothing else. No
//!   path, command, template or target is ever taken from the URL.
//! * The resolved row must be one the launcher would *act* on. Prelude's own
//!   rule is that objects act and commands are handed over, and a web page
//!   must not be able to reach the half that hands over: silently writing the
//!   clipboard, starting an agent, or running a server is not something a link
//!   gets to do.
//! * Anything unrecognised does nothing, visibly. There is no terminal here,
//!   so "visibly" is a notification.

use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SCHEME: &str = "prelude";
const APP_NAME: &str = "Prelude Link.app";

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/\
                          LaunchServices.framework/Support/lsregister";

/// `~/Applications`, which is where the installer already places Ghostty.
///
/// Location is load-bearing: the identical bundle under `/private/tmp`
/// registers its claim and still answers `kLSApplicationNotFoundErr`, and both
/// failures look the same from `open`.
fn apps_dir() -> PathBuf {
    crate::paths::home().join("Applications")
}

pub fn bundle() -> PathBuf {
    apps_dir().join(APP_NAME)
}

pub fn installed() -> bool {
    bundle().is_dir()
}

// ── generating the handler ──────────────────────────────────────────────────

/// The applet's whole program.
///
/// `on open location` is the Apple Event entry point. This is the part that
/// cannot be a shell script: `CFBundleURLTypes` delivers the URL as an Apple
/// Event (`kAEGetURL`) to a running application, never as `argv`, so a bundle
/// whose executable is a script registers correctly, claims the scheme, and
/// then silently never runs.
fn applet_source(binary: &Path) -> String {
    format!(
        "on open location this_URL\n\
         \tdo shell script {} & \" _open-url \" & quoted form of this_URL\n\
         end open location\n",
        applescript_literal(&binary.to_string_lossy())
    )
}

fn applescript_literal(value: &str) -> String {
    format!("\"{}\"", crate::bus::escape(value))
}

fn plist_keys(scheme: &str) -> Vec<String> {
    [
        "Add :CFBundleURLTypes array",
        "Add :CFBundleURLTypes:0 dict",
        "Add :CFBundleURLTypes:0:CFBundleURLName string Prelude",
        "Add :CFBundleURLTypes:0:CFBundleURLSchemes array",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([
        format!("Add :CFBundleURLTypes:0:CFBundleURLSchemes:0 string {scheme}"),
        // No Dock icon and no menu bar: this bundle exists to receive one
        // event and exit, the same reason the panel sets `macos-hidden`.
        "Add :LSUIElement bool true".to_string(),
    ])
    .collect()
}

/// Build and register a handler bundle. `install` is this at the real name.
fn build_at(path: &Path, scheme: &str, binary: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("prelude:// links are available on macOS only".into());
    }
    let dir = path.parent().ok_or_else(|| "no parent directory".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    if path.exists() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("could not replace {}: {e}", path.display()))?;
    }

    let source = dir.join(format!(".prelude-link-{}.applescript", std::process::id()));
    std::fs::write(&source, applet_source(binary))
        .map_err(|e| format!("could not stage the handler: {e}"))?;
    let compiled = crate::exec::capture(
        &["osacompile", "-o", &path.to_string_lossy(), &source.to_string_lossy()],
        Duration::from_secs(30),
    )
    .ok();
    let _ = std::fs::remove_file(&source);
    if !compiled {
        return Err("osacompile could not build the link handler".into());
    }

    let plist = path.join("Contents/Info.plist");
    for key in plist_keys(scheme) {
        if !crate::exec::capture(
            &["/usr/libexec/PlistBuddy", "-c", &key, &plist.to_string_lossy()],
            Duration::from_secs(10),
        )
        .ok()
        {
            return Err(format!("could not add {key} to the handler's Info.plist"));
        }
    }
    register(path);
    Ok(())
}

fn register(path: &Path) {
    let _ = crate::exec::capture(&[LSREGISTER, "-f", &path.to_string_lossy()], Duration::from_secs(20));
}

fn unregister(path: &Path) {
    let _ = crate::exec::capture(&[LSREGISTER, "-u", &path.to_string_lossy()], Duration::from_secs(20));
}

/// Generate `~/Applications/Prelude Link.app` and claim `prelude://`.
///
/// Called by `global install`, so there is no second thing to remember, and
/// undone by `global uninstall` — which must unregister as well as delete, or
/// the scheme stays claimed by a bundle that is gone.
pub fn install() -> Result<String, String> {
    let binary = std::env::current_exe()
        .map_err(|e| format!("could not find this binary: {e}"))?;
    let path = bundle();
    build_at(&path, SCHEME, &binary)?;
    Ok(format!("{SCHEME}:// links handled by {}", path.display()))
}

pub fn uninstall() -> Result<(), String> {
    let path = bundle();
    if !path.exists() {
        return Ok(());
    }
    unregister(&path);
    std::fs::remove_dir_all(&path)
        .map_err(|e| format!("could not remove {}: {e}", path.display()))
}

// ── receiving a URL ─────────────────────────────────────────────────────────

/// `prelude://verb?key=value`, split without a URL crate.
///
/// Deliberately small and total: anything it cannot make sense of comes back
/// as a verb the table does not know, which does nothing.
fn parse(url: &str) -> (String, Vec<(String, String)>) {
    let rest = url
        .strip_prefix(&format!("{SCHEME}://"))
        .or_else(|| url.strip_prefix(&format!("{SCHEME}:")))
        .unwrap_or("");
    let (verb, query) = rest.split_once('?').unwrap_or((rest, ""));
    let verb = percent_decode(verb.trim_matches('/')).to_lowercase();
    let params = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k).to_lowercase(), percent_decode(v))
        })
        .collect();
    (verb, params)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> Option<&'a str> {
    params.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

/// What a link is allowed to do to a row.
///
/// The launcher's own rule, narrowed: an object acts, a command is handed
/// over, and a link may only trigger the first half. `Open`, `Launch` and
/// `OpenUrl` are the three verbs that go to Launch Services and touch nothing
/// of the person's own state; every other default either writes the clipboard,
/// starts a process, or asks a question, and none of those is something a web
/// page gets to cause.
fn may_be_linked(item: &crate::item::Item) -> bool {
    use crate::defaults::{Default_, Verb};
    // `by_kind`, not `on_enter`: `classic_enter` is a preference about what
    // the *launcher's* Enter does, and this has no launcher and no clipboard.
    // Read through `on_enter` it made every deeplink refuse for anybody who
    // had turned copy-everything on — safe, and silently dead.
    matches!(
        crate::defaults::by_kind(item),
        Default_::Act(Verb::Open) | Default_::Act(Verb::Launch) | Default_::Act(Verb::OpenUrl)
    )
}

/// Nothing here has a terminal, so a refusal that only printed would be a
/// silent no-op — the failure this whole table exists to avoid.
///
/// The notification is the reason this is not a one-liner. `bus::post` builds
/// an AppleScript literal and `bus::escape` covers quotes and backslashes but
/// not newlines, so a sentence carrying one would end the `display
/// notification` statement and start another — arbitrary AppleScript, from a
/// URL any web page can navigate to. Nothing derived from the URL reaches this
/// unflattened, and the verb is never echoed at all.
fn refuse(why: &str) -> i32 {
    eprintln!("prelude: {why}");
    crate::bus::post("Prelude", &crate::width::flatten(why));
    2
}

/// A refusal during `--dry-run` is still a refusal, but it must not raise a
/// notification: a check that walks every branch would raise one per branch.
fn refuse_quietly(why: &str, dry: bool) -> i32 {
    if dry {
        eprintln!("prelude: {why}");
        return 2;
    }
    refuse(why)
}

pub fn handle(url: &str) -> i32 {
    act_on(url, false)
}

/// Resolve and check, and say what would happen without doing it.
///
/// The verb table is a security boundary, and a boundary that can only be
/// tested by triggering it is one nobody tests. `--dry-run` walks every
/// refusal and stops one step short of Launch Services.
pub fn describe(url: &str) -> i32 {
    act_on(url, true)
}

fn act_on(url: &str, dry: bool) -> i32 {
    let (verb, params) = parse(url);
    match verb.as_str() {
        "run" => {
            let Some(raw) = param(&params, "alias") else {
                return refuse_quietly("that prelude:// link names no alias", dry);
            };
            // The alias is validated as a *name*, not merely decoded: whatever
            // arrives has to be something `prelude alias` would have accepted,
            // which excludes paths, separators and every kind of injection by
            // construction rather than by filtering.
            let Ok(alias) = crate::compute::normalize_quicklink_key(raw) else {
                return refuse_quietly("that prelude:// link does not name an alias", dry);
            };
            let Some(target) = crate::aliases::target_of(&alias) else {
                return refuse_quietly(&format!("no alias called “{alias}”"), dry);
            };
            let items = crate::cache::gather();
            let Some(item) = items
                .iter()
                .find(|item| crate::favorites::key(item).as_deref() == Some(target.as_str()))
            else {
                return refuse_quietly(&format!("“{alias}” names something that is not here any more"), dry);
            };
            if !may_be_linked(item) {
                return refuse_quietly(
                    &format!("“{alias}” is not something a link may act on — open it from the launcher"),
                    dry,
                );
            }
            if dry {
                println!("would open {} ({})", item.title, item.style().1);
                return 0;
            }
            crate::ui::perform(item, crate::defaults::by_kind(item))
        }
        "" => refuse("that prelude:// link says nothing to do"),
        // The verb is not repeated back. It is arbitrary text from a web page,
        // and the one thing a person needs to know is that nothing happened.
        _ => refuse_quietly("that prelude:// link asks for something this version does not do", dry),
    }
}

// ── proving it end to end ───────────────────────────────────────────────────

/// Build a throwaway handler, fire a URL at it through Launch Services, and
/// see whether it arrives.
///
/// This exists because the failure it covers is invisible any other way: a
/// bundle can be generated correctly, register correctly and claim the scheme
/// in `lsregister -dump` while never receiving anything. Asserting that the
/// scheme is claimed would pass in exactly that case.
pub fn selftest() -> i32 {
    let scheme = format!("prelude-selftest-{}", std::process::id());
    let path = apps_dir().join(format!("Prelude Link Selftest {}.app", std::process::id()));
    let marker = std::env::temp_dir().join(format!("prelude-link-selftest-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);

    // A handler that records the URL it was given, standing in for the binary.
    let source = format!(
        "on open location this_URL\n\
         \tdo shell script \"printf %s \" & quoted form of this_URL & \" > {}\"\n\
         end open location\n",
        crate::bus::escape(&marker.to_string_lossy())
    );
    let staged = apps_dir().join(format!(".prelude-selftest-{}.applescript", std::process::id()));
    let cleanup = |path: &Path, staged: &Path| {
        unregister(path);
        let _ = std::fs::remove_dir_all(path);
        let _ = std::fs::remove_file(staged);
    };
    if std::fs::create_dir_all(apps_dir()).is_err() || std::fs::write(&staged, source).is_err() {
        println!("could not stage a selftest handler");
        return 1;
    }
    if !crate::exec::capture(
        &["osacompile", "-o", &path.to_string_lossy(), &staged.to_string_lossy()],
        Duration::from_secs(30),
    )
    .ok()
    {
        cleanup(&path, &staged);
        println!("osacompile failed");
        return 1;
    }
    let plist = path.join("Contents/Info.plist");
    for key in plist_keys(&scheme) {
        if !crate::exec::capture(
            &["/usr/libexec/PlistBuddy", "-c", &key, &plist.to_string_lossy()],
            Duration::from_secs(10),
        )
        .ok()
        {
            cleanup(&path, &staged);
            println!("PlistBuddy failed on {key}");
            return 1;
        }
    }
    register(&path);

    let sent = format!("{scheme}://run?alias=selftest");
    let _ = crate::exec::capture(&["open", &sent], Duration::from_secs(15));
    // Launch Services starts an application to deliver the event; the wait is
    // for that, not for the handler's own work.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut got = String::new();
    while std::time::Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&marker) {
            if !text.trim().is_empty() {
                got = text.trim().to_string();
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    cleanup(&path, &staged);
    let _ = std::fs::remove_file(&marker);

    if got == sent {
        println!("ok: the handler received {got}");
        0
    } else if got.is_empty() {
        println!("the handler was registered but received nothing");
        1
    } else {
        println!("the handler received {got}, not {sent}");
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Item, Kind};

    #[test]
    fn a_url_is_split_without_trusting_any_of_it() {
        assert_eq!(parse("prelude://run?alias=browser").0, "run");
        assert_eq!(param(&parse("prelude://run?alias=browser").1, "alias"), Some("browser"));
        // Case, the bare form, and percent-encoding all normalise.
        assert_eq!(parse("prelude:RUN?ALIAS=x").0, "run");
        assert_eq!(param(&parse("prelude://run?alias=%E5%9F%BA").1, "alias"), Some("基"));
        // And nothing it cannot understand becomes something it can.
        assert_eq!(parse("prelude://").0, "");
        assert_eq!(parse("http://example.com/run?alias=x").0, "");
        assert_eq!(param(&parse("prelude://run").1, "alias"), None);
    }

    /// The security boundary. A link may trigger the half of the launcher that
    /// acts on objects, and never the half that hands text over or starts
    /// something: a web page must not be able to write the clipboard or run an
    /// agent by being visited.
    #[test]
    fn a_link_may_act_on_an_object_and_nothing_else() {
        let app = Item::new("open -a Zed", Kind::App).title("Zed").put("path", "/Applications/Zed.app");
        let url = Item::new("https://example.test", Kind::Link).put("url", "https://example.test");
        let file = Item::new("/tmp/x.md", Kind::File).put("path", "/tmp/x.md");
        for it in [&app, &url, &file] {
            assert!(may_be_linked(it), "{:?} is an object and acts", it.kind);
        }
        // Everything an alias can also name, and none of it is linkable.
        let skill = Item::new("/review", Kind::Skill).put("name", "review");
        let agent = Item::new("claude", Kind::Agent).put("agent", "claude");
        let mcp = Item::new("claude mcp get drive", Kind::Mcp).put("name", "drive");
        for it in [&skill, &agent, &mcp] {
            assert!(!may_be_linked(it), "{:?} hands over or starts; a link may not", it.kind);
        }
    }

    /// `bus::post` builds an AppleScript literal, and `bus::escape` covers
    /// quotes and backslashes but not newlines. A refusal that repeated the
    /// URL back would therefore let any web page end the `display
    /// notification` statement and start another one.
    #[test]
    fn a_refusal_never_carries_url_text_into_applescript() {
        let (verb, params) = parse("prelude://x%0Adisplay%20dialog%20%22p%22");
        assert!(verb.contains('\n'), "the raw verb really does carry a newline");
        assert!(!crate::width::flatten(&verb).contains('\n'), "flatten is the guard");
        // The alias route is safe by construction rather than by filtering:
        // whatever arrives has to survive `normalize_quicklink_key`.
        let (_, evil) = parse("prelude://run?alias=x%0Adisplay%20dialog");
        let raw = param(&evil, "alias").unwrap();
        assert!(raw.contains('\n'));
        assert!(crate::compute::normalize_quicklink_key(raw).is_err());
        let _ = params;
    }

    #[test]
    fn the_applet_cannot_be_a_shell_script_and_says_which_binary_it_calls() {
        let src = applet_source(Path::new("/usr/local/bin/prelude"));
        assert!(src.starts_with("on open location"), "the Apple Event entry point");
        assert!(src.contains("_open-url"));
        assert!(src.contains("/usr/local/bin/prelude"));
        // A path with a quote in it must not end the AppleScript literal.
        let odd = applet_source(Path::new("/tmp/we\"ird/prelude"));
        assert!(odd.contains("we\\\"ird"));
    }

    #[test]
    fn the_plist_claims_one_scheme_and_shows_no_dock_icon() {
        let keys = plist_keys("prelude").join("\n");
        assert!(keys.contains("CFBundleURLSchemes:0 string prelude"));
        assert!(keys.contains("LSUIElement bool true"));
    }
}
