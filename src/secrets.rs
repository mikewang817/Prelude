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

fn has_prefixed_blob(s: &str, prefix: &str, min: usize, ok: fn(char) -> bool) -> bool {
    s.match_indices(prefix).any(|(i, _)| {
        s[i + prefix.len()..].chars().take_while(|c| ok(*c)).count() >= min
    })
}
