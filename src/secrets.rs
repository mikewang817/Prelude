//! Never index, rank, or transmit anything that looks like a credential.

pub fn looks_secret(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    for needle in [
        "api_key", "api-key", "apikey", "secret", "token", "passwd",
        "password", "bearer ",
    ] {
        if low.contains(needle) {
            return true;
        }
    }
    has_prefixed_blob(s, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || has_prefixed_blob(s, "sk-", 20, |c| c.is_ascii_alphanumeric())
        || has_prefixed_blob(s, "ghp_", 20, |c| c.is_ascii_alphanumeric())
}

/// Stronger signal for content fingerprints. History uses the deliberately
/// broad `looks_secret`; source code routinely contains identifiers such as
/// `NOTICE_TOKEN`, which must not make an entire Skill uncopyable.
pub fn looks_secret_material(s: &str) -> bool {
    let low = s.to_ascii_lowercase();
    if ["api_key", "api-key", "apikey", "passwd", "password", "bearer "]
        .iter().any(|needle| low.contains(needle))
        || has_prefixed_blob(s, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || has_prefixed_blob(s, "sk-", 20, |c| c.is_ascii_alphanumeric())
        || has_prefixed_blob(s, "ghp_", 20, |c| c.is_ascii_alphanumeric())
    {
        return true;
    }
    let Some((field, value)) = s.split_once('=').or_else(|| s.split_once(':')) else { return false };
    let field = field.to_ascii_lowercase();
    if !(field.contains("token") || field.contains("secret")) {
        return false;
    }
    let value = value.trim().trim_matches(['\'', '"']);
    if value.starts_with('$') {
        return true;
    }
    let alnum = value.chars().filter(|c| c.is_ascii_alphanumeric()).count();
    let simple_value = value.len() >= 6
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    simple_value || (value.len() >= 16 && alnum * 10 >= value.len() * 8)
}

fn has_prefixed_blob(s: &str, prefix: &str, min: usize, ok: fn(char) -> bool) -> bool {
    s.match_indices(prefix).any(|(i, _)| {
        s[i + prefix.len()..].chars().take_while(|c| ok(*c)).count() >= min
    })
}
