//! Content identity for Agent capabilities.
//!
//! Hashing whole Skill trees is intentionally a cached background source.
//! The launcher consumes only these small records; it never walks capability
//! directories per keystroke or even on the gather path.

use crate::item::{Item, Kind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
/// What a fingerprint covers. Bump it whenever that answer changes, or two
/// builds will compare hashes that mean different things and call divergent
/// copies identical.
///
/// Unchanged by the fault detection below: a broken or escaping symlink is
/// still hashed as its link text and never as its target. Resolving a target
/// decides whether to *report* it, and contributes nothing to identity.
const SKILL_POLICY: &[u8] = b"skill-tree-v4-vcs-ignored-field-secrets";

/// A Skill's `description` is read by every agent on every load, and Claude's
/// own format caps it near a kilobyte. Past that it is a manual pasted into a
/// header field rather than a description, and is worth saying so.
const MAX_DESCRIPTION: usize = 1024;

/// Named examples, not an inventory. These travel in the Skill cache, so a
/// pathological tree must not be able to grow it without bound.
const MAX_LINK_FAULTS: usize = 32;

/// Schema version of a cached `SkillCopy` record, written by `cache_item` and
/// the only thing a reader may use to decide that an old record is missing a
/// field it now needs.
///
/// It exists because that decision used to be made from `modified`, a value
/// the filesystem gets to choose: a tree whose newest mtime is at or before
/// the epoch hashes to `modified == 0` perfectly legitimately, and a reuse
/// gate spelled `modified > 0` then rejects its own freshly written record
/// every time, re-walking and re-hashing that whole tree on every refresh for
/// ever. A derived value cannot be a sentinel. Bump this whenever a field is
/// added that an older record would report as zero or empty; every record
/// written before it existed reads as `""` and is recomputed exactly once.
pub const RECORD: &str = "2";

struct Fnv(u64);

impl Fnv {
    fn new() -> Self { Self(FNV_OFFSET) }
    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
        // Field boundary: `ab,c` must not equal `a,bc`.
        self.0 ^= 0xff;
        self.0 = self.0.wrapping_mul(FNV_PRIME);
    }
    fn finish(&self) -> String { format!("fnv1a64-v1:{:016x}", self.0) }
}

pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash = Fnv::new();
    hash.bytes(bytes);
    hash.finish()
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SkillCopy {
    pub agent: String,
    pub dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stamp: String,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub sensitive_files: u64,
    #[serde(default)]
    pub unreadable: u64,
    /// Newest modification time anywhere in the effective tree, in seconds
    /// since the epoch.
    ///
    /// A Skill is a directory, so "when did this copy last change" is the
    /// newest mtime under it and not `SKILL.md`'s — a copy whose scripts were
    /// edited yesterday is not a copy that has not moved since last year. The
    /// walk already visits every file for the fingerprint, so taking it here
    /// costs nothing.
    ///
    /// Like every other derived field it must be written into `Item::data` by
    /// `cache_item`: a copy is rebuilt from `data` alone, so a value that is
    /// applied but never recorded is gone by the next cache read. That is the
    /// same mistake `Item::rank` documents for source ranking, and it has
    /// already been paid for once in this codebase.
    ///
    /// It is whatever the filesystem says and nothing else. Zero is a real
    /// answer — an epoch or pre-epoch mtime — and must never be read as "this
    /// record is stale"; that is what `RECORD` is for.
    #[serde(default)]
    pub modified: u64,
    /// Symlinks in the tree whose target does not exist, by path relative to
    /// the Skill directory.
    ///
    /// Worth naming rather than swallowing: a broken link is a Skill that
    /// fails where it is *used*, long after the install that looked fine.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub broken_links: Vec<String>,
    /// Symlinks whose target resolves outside the Skill directory.
    ///
    /// An escaping link means the Skill is not self-contained, so copying or
    /// lending it to another agent silently leaves half of it behind — and
    /// the copy that arrives is broken in a way the fingerprint cannot show,
    /// because the link text matched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub escaping_links: Vec<String>,
}

fn relative<'a>(root: &'a Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

pub fn ignored_path(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    matches!(name.as_ref(), ".git" | ".hg" | ".svn" | ".DS_Store" | "__pycache__")
        || name.ends_with(".pyc")
}

fn mtime(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn note(list: &mut Vec<String>, value: String) {
    if list.len() < MAX_LINK_FAULTS {
        list.push(value);
    }
}

/// Does this path, by name alone, look like credential material?
///
/// One predicate, because the two places that ask — whether a file's *content*
/// contributes only a marker to the fingerprint, and whether a symlink's path
/// may be named in a fault list — are the same question about the same string,
/// and answering it twice is how they came to disagree: the link list kept
/// `credentials.json` and `sub/.env` verbatim while the hash beside it treated
/// exactly those files as private.
///
/// Deliberately unchanged in what it *matches*. Widening it would redact files
/// whose bytes are hashed today, which changes fingerprints for trees that
/// contain them — a `SKILL_POLICY` bump, not a refactor.
fn sensitive_name(rel: &str) -> bool {
    let low = rel.to_ascii_lowercase();
    crate::secrets::looks_secret_material(rel) || low.ends_with("/.env") || low.contains("credential")
}

/// Decide whether one symlink is broken or escaping, without following it.
///
/// The target is resolved to answer that question and then thrown away: it is
/// never hashed and never stored, because a link out of a Skill routinely
/// points into a home directory nobody asked to have indexed. A target that
/// cannot be resolved at all — dangling, or a directory we may not read — is
/// reported as broken, since a link this walk cannot vouch for is exactly the
/// thing a diagnostic exists to say out loud.
fn note_link(real_root: &Path, link: &Path, target: &Path, rel: &str, result: &mut SkillCopy) {
    // The path is inside the Skill and is ordinarily harmless, but a file
    // named after a credential is still a name, so it follows the same rule
    // as the content — literally the same rule, `sensitive_name`.
    let label = if sensitive_name(rel) {
        "<redacted>".to_string()
    } else {
        rel.to_string()
    };
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        link.parent().unwrap_or(Path::new("")).join(target)
    };
    match joined.canonicalize() {
        Err(_) => note(&mut result.broken_links, label),
        Ok(real) => {
            if !real.starts_with(real_root) {
                note(&mut result.escaping_links, label);
            }
        }
    }
}

fn walk(root: &Path, real_root: &Path, path: &Path, hash: &mut Fnv, result: &mut SkillCopy) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        result.unreadable += 1;
        return;
    };
    result.modified = result.modified.max(mtime(&meta));
    let rel = relative(root, path).to_string_lossy();
    hash.bytes(rel.as_bytes());
    if meta.file_type().is_symlink() {
        hash.bytes(b"symlink");
        match std::fs::read_link(path) {
            Ok(target) => {
                hash.bytes(target.to_string_lossy().as_bytes());
                note_link(real_root, path, &target, rel.as_ref(), result);
            }
            Err(_) => result.unreadable += 1,
        }
        return;
    }
    if meta.is_dir() {
        hash.bytes(b"dir");
        let Ok(entries) = std::fs::read_dir(path) else {
            result.unreadable += 1;
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        children.sort();
        for child in children.into_iter().filter(|child| !ignored_path(child)) {
            walk(root, real_root, &child, hash, result);
        }
        return;
    }
    if !meta.is_file() {
        hash.bytes(b"other");
        return;
    }
    result.files += 1;
    result.bytes = result.bytes.saturating_add(meta.len());
    hash.bytes(b"file");
    let name_looks_sensitive = sensitive_name(&rel);
    match std::fs::read(path) {
        Ok(bytes) => {
            if name_looks_sensitive {
                result.sensitive_files += 1;
                hash.bytes(b"sensitive-file-redacted");
            } else if let Ok(text) = std::str::from_utf8(&bytes) {
                let mut redacted = false;
                for line in text.split_inclusive('\n') {
                    if crate::secrets::looks_secret_material(line) {
                        redacted = true;
                        hash.bytes(b"sensitive-line-redacted");
                    } else {
                        hash.bytes(line.as_bytes());
                    }
                }
                if redacted {
                    result.sensitive_files += 1;
                }
            } else {
                hash.bytes(&bytes);
            }
        }
        Err(_) => result.unreadable += 1,
    }
}

fn stamp_walk(root: &Path, path: &Path, hash: &mut Fnv) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    hash.bytes(relative(root, path).to_string_lossy().as_bytes());
    hash.bytes(meta.len().to_string().as_bytes());
    if let Ok(modified) = meta.modified().and_then(|time| {
        time.duration_since(std::time::SystemTime::UNIX_EPOCH)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }) {
        hash.bytes(modified.as_nanos().to_string().as_bytes());
    }
    if meta.file_type().is_symlink() {
        return std::fs::read_link(path).map(|target| hash.bytes(target.to_string_lossy().as_bytes())).is_ok();
    }
    if !meta.is_dir() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(path) else { return false };
    let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    children.sort();
    children.into_iter().filter(|child| !ignored_path(child))
        .all(|child| stamp_walk(root, &child, hash))
}

pub fn skill_stamp(dir: &Path) -> String {
    let mut hash = Fnv::new();
    hash.bytes(SKILL_POLICY);
    if stamp_walk(dir, dir, &mut hash) { hash.finish() } else { String::new() }
}

pub fn hash_skill(agent: &str, dir: &Path) -> SkillCopy {
    let mut result = SkillCopy {
        agent: agent.to_string(),
        dir: dir.to_string_lossy().into_owned(),
        ..SkillCopy::default()
    };
    let mut hash = Fnv::new();
    hash.bytes(SKILL_POLICY);
    // Resolved once: the escape test compares canonical paths, and the root
    // itself is routinely reached through a symlinked home directory, which
    // would make every link in the tree look like it escaped.
    let real_root = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    walk(dir, &real_root, dir, &mut hash, &mut result);
    if result.unreadable == 0 {
        result.fingerprint = hash.finish();
    }
    result
}

pub fn cache_item(copy: &SkillCopy) -> Item {
    Item::new(format!("{}:{}", copy.agent, copy.dir), Kind::Skill)
        .put("record", RECORD)
        .put("agent", &copy.agent)
        .put("dir", &copy.dir)
        .put("fingerprint", &copy.fingerprint)
        .put("stamp", &copy.stamp)
        .put("files", copy.files.to_string())
        .put("bytes", copy.bytes.to_string())
        .put("sensitive_files", copy.sensitive_files.to_string())
        .put("unreadable", copy.unreadable.to_string())
        .put("modified", copy.modified.to_string())
        .put("broken_links", serde_json::to_string(&copy.broken_links).unwrap_or_default())
        .put("escaping_links", serde_json::to_string(&copy.escaping_links).unwrap_or_default())
}

pub fn copy_from_item(item: &Item) -> SkillCopy {
    SkillCopy {
        agent: item.get("agent").to_string(),
        dir: item.get("dir").to_string(),
        fingerprint: item.get("fingerprint").to_string(),
        stamp: item.get("stamp").to_string(),
        files: item.get("files").parse().unwrap_or(0),
        bytes: item.get("bytes").parse().unwrap_or(0),
        sensitive_files: item.get("sensitive_files").parse().unwrap_or(0),
        unreadable: item.get("unreadable").parse().unwrap_or(0),
        modified: item.get("modified").parse().unwrap_or(0),
        broken_links: serde_json::from_str(item.get("broken_links")).unwrap_or_default(),
        escaping_links: serde_json::from_str(item.get("escaping_links")).unwrap_or_default(),
    }
}

/// Was this cached record written by the current build's `cache_item`?
///
/// The field name lives here rather than at the call site, so the reader and
/// the writer cannot drift apart. A record from before the field existed
/// answers `false` and is recomputed once.
pub fn is_current_record(item: &Item) -> bool {
    item.get("record") == RECORD
}

pub fn copies(item: &Item) -> Vec<SkillCopy> {
    serde_json::from_str(item.get("copy_info")).unwrap_or_default()
}

/// What a set of copies of one Skill amounts to: `single`, `identical`,
/// `divergent`, `unknown` or `private-unknown`.
///
/// One implementation because the launcher row, the Quick Look matrix and
/// `doctor skills` are answering the same question, and three copies of these
/// five rules would eventually give three answers about the same skill.
///
/// A copy with redacted private lines can only ever be `private-unknown`:
/// equal fingerprints there mean the *public* parts match, and the parts that
/// were not compared are exactly the ones worth being careful about.
pub fn integrity(copies: &[SkillCopy]) -> &'static str {
    let known = copies.iter().filter(|copy| !copy.fingerprint.is_empty()).count();
    let unique: std::collections::BTreeSet<&str> =
        copies.iter().map(|copy| copy.fingerprint.as_str()).filter(|hash| !hash.is_empty()).collect();
    if known != copies.len() {
        "unknown"
    } else if copies.len() <= 1 {
        "single"
    } else if copies.iter().any(|copy| copy.sensitive_files > 0) {
        "private-unknown"
    } else if unique.len() == 1 {
        "identical"
    } else {
        "divergent"
    }
}

/// Something wrong with one Skill copy, as data.
///
/// A fault is never a panic and never an early `return Vec::new()`: a
/// directory that cannot be read is precisely the case a diagnostic exists
/// for, and answering it with an empty list would report the one broken
/// Skill on the machine as the only clean one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFault {
    /// Stable machine code, safe to match on: `unreadable-dir`,
    /// `missing-skill-md`, `unreadable-skill-md`, `no-frontmatter`,
    /// `unterminated-frontmatter`, `missing-name`, `name-mismatch`,
    /// `missing-description`, `empty-description`, `description-too-long`,
    /// `broken-symlink`, `escaping-symlink`.
    pub code: String,
    /// One line for a person, already redacted where it names a path.
    pub detail: String,
}

fn fault(code: &str, detail: impl Into<String>) -> SkillFault {
    SkillFault { code: code.to_string(), detail: detail.into() }
}

/// Is this directory a Skill an agent will actually load?
///
/// The header is parsed by `sources::agents::parse_front`, the same parser
/// the launcher row displays from. Two parsers over one file eventually
/// disagree, and the disagreement is a row rendering one name beside a
/// diagnostic complaining about another.
pub fn validate_skill(dir: &Path) -> Vec<SkillFault> {
    let shown = crate::paths::tilde(&dir.to_string_lossy());
    if std::fs::read_dir(dir).is_err() {
        return vec![fault("unreadable-dir", format!("{shown} cannot be read as a directory"))];
    }
    let folder = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(md) = ["SKILL.md", "skill.md"].iter().map(|name| dir.join(name)).find(|path| path.exists())
    else {
        return vec![fault("missing-skill-md", "no SKILL.md, so no agent will load this as a Skill")];
    };
    let Ok(text) = std::fs::read_to_string(&md) else {
        return vec![fault("unreadable-skill-md", format!("{shown}: SKILL.md cannot be read"))];
    };
    let front = crate::sources::agents::parse_front(&text);
    if !front.opened {
        // Nothing further can be said: without the opening `---` there is no
        // header to have a name or a description in.
        return vec![fault("no-frontmatter", "SKILL.md does not open with a `---` header block")];
    }
    let mut faults = Vec::new();
    if !front.closed {
        faults.push(fault("unterminated-frontmatter", "the `---` header block is never closed"));
    }
    match front.name.as_deref().map(str::trim) {
        None | Some("") => faults.push(fault("missing-name", "the header declares no `name:`")),
        Some(name) if name != folder => faults.push(fault(
            "name-mismatch",
            format!("the header calls it `{name}` while the directory is `{folder}`"),
        )),
        Some(_) => {}
    }
    match front.desc.as_deref().map(str::trim) {
        None => faults.push(fault("missing-description", "the header declares no `description:`")),
        Some("") => faults.push(fault("empty-description", "`description:` is empty")),
        Some(desc) if desc.chars().count() > MAX_DESCRIPTION => faults.push(fault(
            "description-too-long",
            format!("`description:` is {} characters; agents read it on every load", desc.chars().count()),
        )),
        Some(_) => {}
    }
    faults
}

/// The symlink faults a hash already found, as reportable faults.
pub fn link_faults(copy: &SkillCopy) -> Vec<SkillFault> {
    let mut faults = Vec::new();
    for link in &copy.broken_links {
        faults.push(fault("broken-symlink", format!("{link} points at something that is not there")));
    }
    for link in &copy.escaping_links {
        faults.push(fault(
            "escaping-symlink",
            format!("{link} resolves outside the Skill, so a copy of it would be incomplete"),
        ));
    }
    faults
}

/// One Skill directory as found on disk, before anything is merged.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoveredSkill {
    pub agent: String,
    pub dir: String,
    /// The directory's own name.
    pub folder: String,
    /// The name the header claims, empty when it has none.
    pub declared: String,
}

impl DiscoveredSkill {
    /// What the launcher will call it: the header name, or the folder when
    /// there is none. The same rule `skills_with` merges by, so a collision
    /// reported here is a collision seen there.
    fn effective(&self) -> &str {
        if self.declared.is_empty() { &self.folder } else { &self.declared }
    }
}

/// Every installed Skill directory across every agent root.
///
/// Reads one header per Skill and nothing else. It is not on the gather path
/// and must not be put there.
pub fn discover_skills() -> Vec<DiscoveredSkill> {
    discover_in(&crate::sources::agents::skill_dirs())
}

/// The same, over a given set of roots.
///
/// The roots are a parameter for one reason: a function that calls
/// `skill_dirs()` itself can only ever be tested against whatever the machine
/// running the tests happens to have installed — which on a clean checkout is
/// nothing, so the discovery rules (skip non-directories, skip dotted names,
/// prefer `SKILL.md` over `skill.md`, fall back to the folder name) went
/// entirely unexercised.
fn discover_in(roots: &[(PathBuf, &'static str)]) -> Vec<DiscoveredSkill> {
    let mut found = Vec::new();
    for (root, agent) in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        dirs.sort();
        for dir in dirs {
            let folder = dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            // e.g. codex's internal .system directory
            if !dir.is_dir() || folder.starts_with('.') {
                continue;
            }
            let declared = ["SKILL.md", "skill.md"]
                .iter()
                .map(|name| dir.join(name))
                .find(|path| path.exists())
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| crate::sources::agents::parse_front(&text).name)
                .unwrap_or_default();
            found.push(DiscoveredSkill {
                agent: agent.to_string(),
                dir: dir.to_string_lossy().into_owned(),
                folder,
                declared: declared.trim().to_string(),
            });
        }
    }
    found
}

/// Two Skills, under one agent, that cannot both be what they claim to be.
///
/// Two rules, not three. `folder-name-mismatch` used to be reported here and
/// has moved to `validate_skill`, which already had it as the `name-mismatch`
/// fault: a header disagreeing with its own directory is one directory being
/// wrong about itself, which is the definition of a `SkillFault` and not of a
/// collision. Filing it here meant a "collision" with one path in it, and two
/// codes for one condition that a consumer had to know were the same.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collision {
    /// `case` or `duplicate-name`.
    pub kind: String,
    /// The agent whose skills directory holds every path below. A collision is
    /// always within one agent, so this is a field rather than a prefix glued
    /// onto `name`: a consumer that wants to say "codex has two `review`s"
    /// should not have to parse it back out of a display string.
    pub agent: String,
    /// The contested name, lowercased. Always just the name.
    pub name: String,
    pub paths: Vec<String>,
}

/// Name conflicts among discovered Skill directories.
///
/// Pure over what discovery found, so the rules can be tested without a
/// machine that happens to have the conflict installed.
pub fn collisions(found: &[DiscoveredSkill]) -> Vec<Collision> {
    use std::collections::BTreeMap;
    let mut out = Vec::new();
    // Folder names differing only by case, *within one agent's root*. macOS
    // filesystems are usually case-insensitive, so `Foo` and `foo` in one
    // directory are one directory here and two on a colleague's machine: an
    // install that reports "already installed" on this laptop overwrites
    // somebody's work on that one. Two agents spelling a shared skill
    // differently is not that, and cannot become that: `~/.claude/skills/Deploy`
    // and `~/.codex/skills/deploy` are different directories in different
    // parents on every filesystem there is.
    let mut by_lower: BTreeMap<(&str, String), Vec<&DiscoveredSkill>> = BTreeMap::new();
    for skill in found {
        by_lower.entry((skill.agent.as_str(), skill.folder.to_lowercase())).or_default().push(skill);
    }
    for ((agent, lower), group) in &by_lower {
        let spellings: std::collections::BTreeSet<&str> =
            group.iter().map(|skill| skill.folder.as_str()).collect();
        if spellings.len() > 1 {
            out.push(Collision {
                kind: "case".into(),
                agent: (*agent).to_string(),
                name: lower.clone(),
                paths: group.iter().map(|skill| skill.dir.clone()).collect(),
            });
        }
    }
    // One agent, two directories claiming one name. The launcher merges by
    // name, so the loser is invisible in the list — and which of them the
    // agent itself loads is the agent's business, not something a row can say.
    let mut by_agent: BTreeMap<(&str, String), Vec<&DiscoveredSkill>> = BTreeMap::new();
    for skill in found {
        by_agent.entry((skill.agent.as_str(), skill.effective().to_lowercase())).or_default().push(skill);
    }
    for ((agent, name), group) in &by_agent {
        if group.len() > 1 {
            out.push(Collision {
                kind: "duplicate-name".into(),
                agent: (*agent).to_string(),
                name: name.clone(),
                paths: group.iter().map(|skill| skill.dir.clone()).collect(),
            });
        }
    }
    out
}

/// One copy's integrity state, for a diagnostic rather than a row.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillCopyHealth {
    pub agent: String,
    pub dir: String,
    /// Empty when the tree could not be read completely, which is what makes
    /// the whole Skill `unknown` rather than identical.
    pub fingerprint: String,
    pub modified: u64,
    pub files: u64,
    pub bytes: u64,
    /// Files whose content contributed only a redaction marker.
    pub sensitive_files: u64,
    pub unreadable: u64,
    pub faults: Vec<SkillFault>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillHealth {
    pub name: String,
    /// `single`, `identical`, `divergent`, `unknown` or `private-unknown`.
    pub integrity: String,
    pub copies: Vec<SkillCopyHealth>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillReport {
    pub skills: Vec<SkillHealth>,
    pub collisions: Vec<Collision>,
}

/// Everything `prelude doctor skills` renders, computed fresh.
///
/// **This hashes Skill trees.** It is a command a person runs and waits for:
/// never `gather`, never the per-keystroke helper, never a background tick
/// dressed up as one. The launcher reads the small `skill-hashes` cache
/// instead, and the entire point of that cache is that nothing on the launch
/// path walks a capability directory.
///
/// It deliberately does not read that cache either. A cached hash is a
/// statement about what was true up to thirty seconds ago, and "is this Skill
/// intact *now*" is the one question the person running a diagnostic is
/// asking.
pub fn skill_diagnostics() -> SkillReport {
    diagnose(&discover_skills())
}

/// The same, over a given set of Skill directories, so the assembly can be
/// tested against fixtures rather than against whatever this machine happens
/// to have installed.
fn diagnose(found: &[DiscoveredSkill]) -> SkillReport {
    use std::collections::BTreeMap;
    let mut merged: BTreeMap<String, (Vec<SkillCopy>, Vec<SkillCopyHealth>)> = BTreeMap::new();
    for skill in found {
        let dir = Path::new(&skill.dir);
        let copy = hash_skill(&skill.agent, dir);
        let mut faults = validate_skill(dir);
        faults.extend(link_faults(&copy));
        let health = SkillCopyHealth {
            agent: skill.agent.clone(),
            dir: skill.dir.clone(),
            fingerprint: copy.fingerprint.clone(),
            modified: copy.modified,
            files: copy.files,
            bytes: copy.bytes,
            sensitive_files: copy.sensitive_files,
            unreadable: copy.unreadable,
            faults,
        };
        let entry = merged.entry(skill.effective().to_string()).or_default();
        entry.0.push(copy);
        entry.1.push(health);
    }
    SkillReport {
        skills: merged
            .into_iter()
            .map(|(name, (copies, health))| SkillHealth {
                name,
                integrity: integrity(&copies).to_string(),
                copies: health,
            })
            .collect(),
        collisions: collisions(found),
    }
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct McpVariant {
    pub agent: String,
    pub health: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub transport: String,
    #[serde(default)]
    pub health_checked_at: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub public_definition: serde_json::Value,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub portable: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tools_status: String,
    #[serde(default)]
    pub tools_checked_at: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<crate::mcp_tools::Tool>,
}

pub fn mcp_variants(item: &Item) -> Vec<McpVariant> {
    serde_json::from_str(item.get("variants")).unwrap_or_default()
}

fn flatten_json(prefix: &str, value: &serde_json::Value, out: &mut std::collections::BTreeMap<String, String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                flatten_json(&path, value, out);
            }
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                flatten_json(&format!("{prefix}[{index}]"), value, out);
            }
        }
        _ => {
            out.insert(prefix.to_string(), value.to_string());
        }
    }
}

pub fn mcp_definition_diff(item: &Item) -> Vec<String> {
    let variants = mcp_variants(item);
    if variants.len() < 2 {
        return vec!["only one owner definition is available".into()];
    }
    let mut flattened = Vec::new();
    let mut paths = std::collections::BTreeSet::new();
    for variant in &variants {
        let mut fields = std::collections::BTreeMap::new();
        flatten_json("", &variant.public_definition, &mut fields);
        paths.extend(fields.keys().cloned());
        flattened.push(fields);
    }
    let mut lines = Vec::new();
    for path in paths {
        let values: Vec<Option<&String>> = flattened.iter().map(|fields| fields.get(&path)).collect();
        if values.windows(2).all(|pair| pair[0] == pair[1]) {
            continue;
        }
        lines.push(path.clone());
        for (variant, value) in variants.iter().zip(values) {
            lines.push(format!(
                "  {:<10} {}",
                variant.agent,
                value.map(String::as_str).unwrap_or("<absent>")
            ));
        }
        if lines.len() >= 100 {
            lines.push("… diff truncated".into());
            break;
        }
    }
    if lines.is_empty() {
        lines.push(if variants.iter().any(|variant| variant.sensitive) {
            "public structures match; private values were not compared".into()
        } else {
            "public structures match".into()
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary tree that removes itself, however the test ends.
    ///
    /// `Drop` runs while a panic unwinds, so a failing test leaves nothing
    /// behind in `/tmp` — which matters here because the next run would
    /// otherwise start from one test's debris and fail for the wrong reason.
    struct Fixture(PathBuf);

    impl std::ops::Deref for Fixture {
        type Target = Path;
        fn deref(&self) -> &Path { &self.0 }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    /// Never the user's own `~/.claude/skills`: a test that hashed those
    /// would be slow, machine-dependent, and one careless edit away from
    /// writing into somebody's work.
    ///
    /// The pid alone is not unique enough: `cargo test` runs every test in one
    /// process, on threads that share it, so two tests passing the same name —
    /// or one test run twice concurrently — would be writing into each other's
    /// tree. The counter makes the name unique per call rather than per run.
    fn fixture(name: &str) -> Fixture {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("prelude-{name}-{}-{serial}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Fixture(root)
    }

    fn codes(faults: &[SkillFault]) -> Vec<&str> {
        faults.iter().map(|fault| fault.code.as_str()).collect()
    }

    #[test]
    fn symlink_faults_are_recorded_without_the_hash_ever_following_a_link() {
        use std::os::unix::fs::symlink;
        let root = fixture("skill-links");
        let outside = root.join("outside.txt");
        std::fs::write(&outside, "one").unwrap();
        let skill = root.join("linked");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: linked\ndescription: d\n---\n").unwrap();
        symlink(&outside, skill.join("escapes")).unwrap();
        symlink("nowhere.txt", skill.join("dangles")).unwrap();
        symlink("SKILL.md", skill.join("inside")).unwrap();

        let copy = hash_skill("claude", &skill);
        assert_eq!(copy.broken_links, vec!["dangles".to_string()]);
        assert_eq!(copy.escaping_links, vec!["escapes".to_string()]);
        assert!(copy.modified > 0, "the newest mtime in the tree is the copy's mtime");
        assert!(!copy.fingerprint.is_empty(), "faults are data; they do not make a tree unreadable");
        assert_eq!(link_faults(&copy).len(), 2);
        assert_eq!(codes(&link_faults(&copy)), ["broken-symlink", "escaping-symlink"]);

        // The target's *content* is not part of the Skill: hashing it would
        // mean a Skill whose identity changes when a file it does not own is
        // edited, and would drag that file into the fingerprint uninspected.
        std::fs::write(&outside, "two").unwrap();
        assert_eq!(hash_skill("claude", &skill).fingerprint, copy.fingerprint);
        // The link text is, so re-aiming it is a change.
        std::fs::remove_file(skill.join("escapes")).unwrap();
        symlink(root.join("elsewhere.txt"), skill.join("escapes")).unwrap();
        assert_ne!(hash_skill("claude", &skill).fingerprint, copy.fingerprint);
    }

    #[test]
    fn copy_fields_survive_the_cache_round_trip() {
        // A value applied but never written into `data` is gone by the next
        // read, because a copy is rebuilt from `data` alone.
        let copy = SkillCopy {
            agent: "claude".into(),
            dir: "/tmp/skills/x".into(),
            fingerprint: "fnv1a64-v1:1".into(),
            stamp: "fnv1a64-v1:2".into(),
            files: 3,
            bytes: 44,
            sensitive_files: 1,
            unreadable: 0,
            modified: 1_754_600_000,
            broken_links: vec!["scripts/gone".into()],
            escaping_links: vec!["ref".into()],
        };
        let back = copy_from_item(&cache_item(&copy));
        assert_eq!(back.modified, copy.modified);
        assert_eq!(back.broken_links, copy.broken_links);
        assert_eq!(back.escaping_links, copy.escaping_links);
        assert_eq!(back.fingerprint, copy.fingerprint);
    }

    #[test]
    fn skill_validation_answers_with_faults_rather_than_silence() {
        let root = fixture("skill-validate");
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(codes(&validate_skill(&empty)), ["missing-skill-md"]);

        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("SKILL.md"), "# plain\n\nno header at all\n").unwrap();
        assert_eq!(codes(&validate_skill(&plain)), ["no-frontmatter"]);

        let wrong = root.join("wrong");
        std::fs::create_dir_all(&wrong).unwrap();
        std::fs::write(wrong.join("SKILL.md"), "---\nname: other\n").unwrap();
        assert_eq!(
            codes(&validate_skill(&wrong)),
            ["unterminated-frontmatter", "name-mismatch", "missing-description"],
        );

        let empty_desc = root.join("empty_desc");
        std::fs::create_dir_all(&empty_desc).unwrap();
        std::fs::write(empty_desc.join("SKILL.md"), "---\ndescription:\n---\n").unwrap();
        assert_eq!(codes(&validate_skill(&empty_desc)), ["missing-name", "empty-description"]);

        let long = root.join("long");
        std::fs::create_dir_all(&long).unwrap();
        let text = format!("---\nname: long\ndescription: {}\n---\n", "x".repeat(MAX_DESCRIPTION + 1));
        std::fs::write(long.join("SKILL.md"), text).unwrap();
        assert_eq!(codes(&validate_skill(&long)), ["description-too-long"]);

        let good = root.join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("SKILL.md"), "---\nname: good\ndescription: does a thing\n---\n").unwrap();
        assert!(validate_skill(&good).is_empty());

        // An editor's UTF-8 BOM sits in front of the opening `---`. Every
        // agent loads this Skill; a diagnostic that called it broken would be
        // reporting on the editor, not on the Skill.
        let bom = root.join("bom");
        std::fs::create_dir_all(&bom).unwrap();
        std::fs::write(bom.join("SKILL.md"), "\u{feff}---\nname: bom\ndescription: fine\n---\n").unwrap();
        assert!(validate_skill(&bom).is_empty(), "a BOM is not a missing header");

        // An unreadable directory is the case a diagnostic exists for; a
        // clean answer there would report the one broken Skill as the only
        // healthy one.
        assert_eq!(codes(&validate_skill(&root.join("not-there"))), ["unreadable-dir"]);
    }

    #[test]
    fn name_collisions_are_found_within_one_agent_root_and_only_there() {
        let skill = |agent: &str, dir: &str, folder: &str, declared: &str| DiscoveredSkill {
            agent: agent.into(),
            dir: dir.into(),
            folder: folder.into(),
            declared: declared.into(),
        };
        let found = vec![
            // Two spellings in one directory: one directory on this machine,
            // two on a case-sensitive one. This is the collision.
            skill("claude", "/s/claude/Deploy", "Deploy", "Deploy"),
            skill("claude", "/s/claude/deploy", "deploy", "deploy"),
            // The same two spellings under *different* agents are two
            // directories in two parents, and cannot collide anywhere.
            skill("codex", "/s/codex/deploy", "deploy", "deploy"),
            skill("pi", "/s/pi/Deploy", "Deploy", "Deploy"),
            // One agent, two directories claiming one name.
            skill("claude", "/s/claude/review-old", "review-old", "review"),
            skill("claude", "/s/claude/review", "review", "review"),
        ];
        let found = collisions(&found);
        let case: Vec<&Collision> = found.iter().filter(|c| c.kind == "case").collect();
        assert_eq!(case.len(), 1, "only the pair sharing a parent directory collides");
        assert_eq!(case[0].agent, "claude");
        assert_eq!(case[0].name, "deploy");
        assert_eq!(case[0].paths, vec!["/s/claude/Deploy", "/s/claude/deploy"]);

        let duplicate: Vec<&Collision> = found.iter().filter(|c| c.kind == "duplicate-name").collect();
        // `Deploy`/`deploy` under one agent are both: two spellings of one
        // directory name, and two directories claiming one loaded name.
        assert_eq!(duplicate.len(), 2, "one agent cannot load two Skills under one name");
        let review = duplicate.iter().find(|c| c.name == "review").expect("review");
        assert_eq!(review.agent, "claude", "the agent is a field, not a prefix on the name");
        assert_eq!(review.paths, vec!["/s/claude/review-old", "/s/claude/review"], "discovery order");
        assert!(
            duplicate.iter().all(|c| c.agent == "claude"),
            "the same name under two agents is two agents having the skill, not a conflict",
        );

        // A header disagreeing with its own directory is one directory wrong
        // about itself, and is reported by `validate_skill` as `name-mismatch`.
        assert!(found.iter().all(|c| c.kind == "case" || c.kind == "duplicate-name"));
        assert!(collisions(&[]).is_empty());
    }

    #[test]
    fn discovery_reads_fixture_roots_rather_than_whatever_is_installed() {
        let root = fixture("skill-discover");
        let claude = root.join("claude");
        let codex = root.join("codex");
        let make = |dir: &Path, name: &str, body: Option<&str>| {
            let sub = dir.join(name);
            std::fs::create_dir_all(&sub).unwrap();
            if let Some(body) = body {
                std::fs::write(sub.join("SKILL.md"), body).unwrap();
            }
            sub
        };
        make(&claude, "Deploy", Some("---\nname: Deploy\ndescription: d\n---\n"));
        // No header at all: the folder name is what it is called.
        make(&claude, "bare", None);
        // A lowercase `skill.md`, and a declared name that is not the folder.
        let odd = make(&codex, "odd", None);
        std::fs::write(odd.join("skill.md"), "---\nname: renamed\ndescription: d\n---\n").unwrap();
        // Skipped: a dotted directory (codex's own `.system`) and a plain file.
        make(&codex, ".system", Some("---\nname: system\n---\n"));
        std::fs::write(codex.join("loose.md"), "not a skill").unwrap();

        let roots = vec![(claude.clone(), "claude"), (codex.clone(), "codex"), (root.join("gone"), "pi")];
        let found = discover_in(&roots);
        let names: Vec<(&str, &str, &str)> = found
            .iter()
            .map(|skill| (skill.agent.as_str(), skill.folder.as_str(), skill.declared.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![("claude", "Deploy", "Deploy"), ("claude", "bare", ""), ("codex", "odd", "renamed")],
            "dotted directories, loose files and a missing root are all silently nothing",
        );
        assert_eq!(found[1].effective(), "bare", "no header means the folder name");
        assert_eq!(found[2].effective(), "renamed");

        // …and the same fixture through the whole diagnostic, which is the
        // other half that could never be tested while it called `skill_dirs`.
        // (The case rule is pinned above on data: two folder spellings cannot
        // both exist in one directory on a case-insensitive macOS volume,
        // which is the very machine the rule warns about.)
        let report = diagnose(&found);
        assert!(report.collisions.is_empty(), "three distinct names under two agents");
        let renamed = report.skills.iter().find(|skill| skill.name == "renamed").expect("renamed");
        assert_eq!(renamed.integrity, "single");
        assert_eq!(codes(&renamed.copies[0].faults), ["name-mismatch"]);
        let bare = report.skills.iter().find(|skill| skill.name == "bare").expect("bare");
        assert_eq!(codes(&bare.copies[0].faults), ["missing-skill-md"]);
    }

    #[test]
    fn diagnostics_carry_faults_and_divergence_per_copy() {
        use std::os::unix::fs::symlink;
        let root = fixture("skill-doctor");
        let make = |agent: &str, folder: &str, body: &str| {
            let dir = root.join(agent).join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("SKILL.md"), body).unwrap();
            DiscoveredSkill {
                agent: agent.to_string(),
                dir: dir.to_string_lossy().into_owned(),
                folder: folder.to_string(),
                declared: crate::sources::agents::parse_front(body).name.unwrap_or_default(),
            }
        };
        let mut found = vec![
            make("claude", "deploy", "---\nname: deploy\ndescription: ships it\n---\nleft\n"),
            make("codex", "deploy", "---\nname: deploy\ndescription: ships it\n---\nright\n"),
            make("pi", "broken", "no header here\n"),
        ];
        // A secret in one copy of a third Skill: equal public fingerprints
        // must not be reported as identical when the differing lines were the
        // ones that were never compared.
        found.push(make("claude", "keys", "---\nname: keys\ndescription: d\nAPI_KEY: sk-aaaaaaaaaaaaaaaaaaaa\n---\n"));
        found.push(make("codex", "keys", "---\nname: keys\ndescription: d\nAPI_KEY: sk-bbbbbbbbbbbbbbbbbbbb\n---\n"));
        symlink("gone.txt", root.join("pi/broken/dangling")).unwrap();

        let report = diagnose(&found);
        let by_name = |name: &str| {
            report.skills.iter().find(|skill| skill.name == name).expect("skill in report").clone()
        };
        let deploy = by_name("deploy");
        assert_eq!(deploy.integrity, "divergent", "same header, different body");
        assert_eq!(deploy.copies.len(), 2);
        assert!(deploy.copies.iter().all(|copy| copy.faults.is_empty() && copy.modified > 0));

        let keys = by_name("keys");
        assert_eq!(keys.integrity, "private-unknown");
        assert!(keys.copies.iter().all(|copy| copy.sensitive_files == 1));

        let broken = by_name("broken");
        assert_eq!(broken.integrity, "single");
        assert_eq!(codes(&broken.copies[0].faults), ["no-frontmatter", "broken-symlink"]);
        assert!(report.collisions.is_empty(), "distinct names in distinct roots do not collide");
    }

    #[test]
    fn one_rule_decides_a_private_path_for_both_the_hash_and_the_link_list() {
        use std::os::unix::fs::symlink;
        // The four names the two rules used to disagree about. `walk` redacted
        // the file's bytes for the first two; `note_link` printed all four
        // verbatim into a list that travels into the skill cache, into every
        // Skill row's `copy_info` and into `control --json`.
        assert!(sensitive_name("sub/.env"));
        assert!(sensitive_name("credentials.json"));
        assert!(sensitive_name("api_key.txt"));
        // These two the shared rule does not match *by name*, and deliberately
        // still does not: their bytes are hashed today, so redacting them by
        // name would change the fingerprint of every tree that holds one —
        // that is a `SKILL_POLICY` bump, not a refactor. `walk` catches them
        // by content, which is where the credential actually is.
        assert!(!sensitive_name(".env"), "a root-level .env: matched by its lines, not its name");
        assert!(!sensitive_name("aws_secret_access_key"));
        assert!(!sensitive_name("SKILL.md") && !sensitive_name("scripts/run.sh"));

        let root = fixture("skill-private-names");
        let skill = root.join("named");
        std::fs::create_dir_all(skill.join("sub")).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\nname: named\ndescription: d\n---\n").unwrap();
        for name in ["api_key.txt", "credentials.json", "notes.md", "sub/.env"] {
            symlink("nowhere", skill.join(name)).unwrap();
        }
        let copy = hash_skill("claude", &skill);
        assert_eq!(
            copy.broken_links,
            vec!["<redacted>", "<redacted>", "notes.md", "<redacted>"],
            "a path the hash treats as private is a marker in the fault list too",
        );
    }

    #[test]
    fn a_cached_record_carries_its_own_schema_version() {
        // The reuse gate in `sources::agents` reads this and nothing derived:
        // `modified` is allowed to be 0, and a record from before the field
        // existed is the only thing that must be recomputed.
        let item = cache_item(&SkillCopy { modified: 0, ..SkillCopy::default() });
        assert_eq!(item.get("record"), RECORD);
        assert!(is_current_record(&item));
        let mut legacy = item.clone();
        legacy.data.remove("record");
        assert!(!is_current_record(&legacy));
    }

    #[test]
    fn integrity_never_calls_a_private_copy_identical() {
        let hashed = |hash: &str, sensitive: u64| SkillCopy {
            fingerprint: hash.into(),
            sensitive_files: sensitive,
            ..SkillCopy::default()
        };
        assert_eq!(integrity(&[hashed("a", 0)]), "single");
        assert_eq!(integrity(&[hashed("a", 0), hashed("a", 0)]), "identical");
        assert_eq!(integrity(&[hashed("a", 0), hashed("b", 0)]), "divergent");
        assert_eq!(integrity(&[hashed("a", 0), hashed("", 0)]), "unknown");
        assert_eq!(integrity(&[hashed("a", 1), hashed("a", 0)]), "private-unknown");
    }
}
