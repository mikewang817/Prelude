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
    // Written with a space these are ordinary English, so they are matched as
    // whole phrases: "the private keyboard shortcut" is a sentence, and this
    // test also decides what is kept out of shell history and the clipboard.
    for phrase in ["api key", "private key"] {
        if contains_phrase(&low, phrase) {
            return true;
        }
    }
    has_prefixed_blob(s, "AKIA", 16, |c| c.is_ascii_uppercase() || c.is_ascii_digit())
        || has_secret_key(s)
        || has_prefixed_blob(s, "ghp_", 20, |c| c.is_ascii_alphanumeric())
        // Fine-grained GitHub tokens carry underscores inside the blob.
        || has_prefixed_blob(s, "github_pat_", 20, |c| c.is_ascii_alphanumeric() || c == '_')
        // Google API keys: AIza + 35 characters, and nothing else looks like it.
        || has_prefixed_blob(s, "AIza", 30, |c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        || has_jwt(s)
        || has_url_password(s)
}

/// `sk-…` and `sk_…` secret keys, including the ones that carry a separator.
///
/// A plain "twenty unbroken alphanumerics after `sk-`" test misses the two
/// shapes most often pasted: OpenAI's project keys (`sk-proj-…`) stop the run
/// dead at the hyphen four characters in, and Stripe's `sk_live_…` never had a
/// hyphen to begin with. Both sailed through the filter that exists to keep
/// them out of shell history and the clipboard.
///
/// Allowing separators inside the blob costs a boundary check, and the reason
/// is `risk-`: `match_indices` finds `sk-` in the middle of an ordinary word,
/// so without the check "risk-management-and-compliance" reads as a
/// credential. Requiring the prefix to start a token is what buys the
/// separators, and it removes that false positive rather than adding one —
/// a generated key is a token, never the tail of an English word.
fn has_secret_key(s: &str) -> bool {
    ["sk-", "sk_"].iter().any(|prefix| {
        s.match_indices(prefix).any(|(i, _)| {
            if s[..i].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric()) {
                return false;
            }
            let blob: &str = {
                let rest = &s[i + prefix.len()..];
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
                    .unwrap_or(rest.len());
                &rest[..end]
            };
            blob.len() >= 20
        })
    })
}

/// `needle` surrounded by something that is not a letter or a digit.
///
/// Only for phrases whose words are common on their own; a substring test on
/// "private key" eats "private keyboard".
fn contains_phrase(low: &str, needle: &str) -> bool {
    low.match_indices(needle).any(|(i, _)| {
        let before = low[..i].chars().next_back();
        let after = low[i + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric()) && !after.is_some_and(|c| c.is_alphanumeric())
    })
}

/// How many base64url characters this string starts with.
fn b64url_run(s: &str) -> usize {
    s.chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .count()
}

/// A JSON Web Token: three base64url segments, the first of which encodes a
/// JSON header and therefore always begins `eyJ`. A bearer token pasted on its
/// own line carries no keyword at all, so nothing else here would catch it.
fn has_jwt(s: &str) -> bool {
    s.match_indices("eyJ").any(|(i, _)| {
        let rest = &s[i..];
        let header = b64url_run(rest);
        // All base64url characters are ASCII, so the run length is also a byte
        // offset.
        if header < 12 || !rest[header..].starts_with('.') {
            return false;
        }
        let after_header = &rest[header + 1..];
        let payload = b64url_run(after_header);
        if payload < 8 || !after_header[payload..].starts_with('.') {
            return false;
        }
        b64url_run(&after_header[payload + 1..]) >= 4
    })
}

/// `scheme://user:pass@host` — a credential in a URL's authority needs no
/// keyword such as "token" to be dangerous, and a connection string is the
/// most common way one is written down. A bare `ssh://git@host` is not a
/// credential and is deliberately not matched: the password half is required.
fn has_url_password(s: &str) -> bool {
    s.match_indices("://").any(|(i, _)| {
        let rest = &s[i + 3..];
        let authority = rest.split(['/', '?', '#', ' ']).next().unwrap_or("");
        authority
            .rsplit_once('@')
            .is_some_and(|(userinfo, host)| !host.is_empty() && userinfo.contains(':'))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shapes a person actually pastes. Each is checked on its own,
    /// because this test used to assert only `sk-…` — the one shape the code
    /// already caught — and certified a property it did not have.
    #[test]
    fn every_credential_shape_a_person_pastes_is_recognised() {
        for s in [
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "DATABASE_URL=postgres://admin:hunter2@db.internal/app",
            "export GITHUB_PAT=github_pat_11ABCDEFG0123456789abcdefghijklmnopqrstuvwxyz",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "my API key is below",
            "sk-0123456789abcdefghijklmno",
            "AKIAIOSFODNN7EXAMPLE",
            // The two shapes a separator used to hide. A run of alphanumerics
            // stops dead at the hyphen four characters into `sk-proj-`, and
            // Stripe's keys never had a hyphen at all, so both walked past a
            // filter written to keep exactly them out of shell history.
            "OPENAI_API_KEY=sk-proj-0123456789abcdefghijklmnopqrstuv",
            "sk_live_0123456789abcdefghijklmnop",
        ] {
            assert!(looks_secret(s), "{s}");
        }
    }

    /// This test also decides what is kept out of shell history and the
    /// clipboard, so a phrase made of two common words has to be a phrase.
    #[test]
    fn ordinary_writing_about_keys_is_not_eaten() {
        for s in [
            "the private keyboard shortcut",
            "api keyboard layout",
            "git clone ssh://git@github.com/me/app.git",
            "curl https://example.com/api/v1/users",
            "open https://docs.rs/serde",
            "cargo build --release",
            // `match_indices` finds `sk-` inside an ordinary word. Letting a
            // key blob carry separators would have made this a credential
            // without the token-start rule that bought the separators.
            "risk-management-and-compliance-review",
            "the whisk-and-bowl-method-explained",
        ] {
            assert!(!looks_secret(s), "{s}");
        }
    }
}
