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
const SKILL_POLICY: &[u8] = b"skill-tree-v4-vcs-ignored-field-secrets";

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
}

fn relative<'a>(root: &'a Path, path: &'a Path) -> &'a Path {
    path.strip_prefix(root).unwrap_or(path)
}

pub fn ignored_path(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    matches!(name.as_ref(), ".git" | ".hg" | ".svn" | ".DS_Store" | "__pycache__")
        || name.ends_with(".pyc")
}

fn walk(root: &Path, path: &Path, hash: &mut Fnv, result: &mut SkillCopy) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        result.unreadable += 1;
        return;
    };
    let rel = relative(root, path).to_string_lossy();
    hash.bytes(rel.as_bytes());
    if meta.file_type().is_symlink() {
        hash.bytes(b"symlink");
        match std::fs::read_link(path) {
            Ok(target) => hash.bytes(target.to_string_lossy().as_bytes()),
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
            walk(root, &child, hash, result);
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
    let low_name = rel.to_ascii_lowercase();
    let name_looks_sensitive = crate::secrets::looks_secret_material(&rel)
        || low_name.ends_with("/.env")
        || low_name.contains("credential");
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
    walk(dir, dir, &mut hash, &mut result);
    if result.unreadable == 0 {
        result.fingerprint = hash.finish();
    }
    result
}

pub fn cache_item(copy: &SkillCopy) -> Item {
    Item::new(format!("{}:{}", copy.agent, copy.dir), Kind::Skill)
        .put("agent", &copy.agent)
        .put("dir", &copy.dir)
        .put("fingerprint", &copy.fingerprint)
        .put("stamp", &copy.stamp)
        .put("files", copy.files.to_string())
        .put("bytes", copy.bytes.to_string())
        .put("sensitive_files", copy.sensitive_files.to_string())
        .put("unreadable", copy.unreadable.to_string())
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
    }
}

pub fn copies(item: &Item) -> Vec<SkillCopy> {
    serde_json::from_str(item.get("copy_info")).unwrap_or_default()
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct McpVariant {
    pub agent: String,
    pub health: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub portable: bool,
}

pub fn mcp_variants(item: &Item) -> Vec<McpVariant> {
    serde_json::from_str(item.get("variants")).unwrap_or_default()
}
