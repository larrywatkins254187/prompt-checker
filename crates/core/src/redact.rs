//! Masks secrets (API keys, tokens, passwords, private keys) before text leaves the
//! machine for analysis. The local database always keeps the original text.

use once_cell::sync::Lazy;
use regex::{Captures, Regex};

pub const MASK: &str = "[REDACTED]";

/// Patterns whose whole match is a secret.
static WHOLE: Lazy<Vec<Regex>> = Lazy::new(|| {
    [
        // PEM private keys
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        // Anthropic / OpenAI style keys
        r"\bsk-(?:ant-)?[A-Za-z0-9_\-]{20,}",
        // GitHub tokens
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{30,}",
        r"\bgithub_pat_[A-Za-z0-9_]{40,}",
        // AWS access key id
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        // Google API key
        r"\bAIza[0-9A-Za-z_\-]{35}",
        // Slack tokens
        r"\bxox[abprs]-[A-Za-z0-9\-]{10,}",
        // Stripe keys
        r"\b(?:sk|rk|pk)_(?:live|test)_[A-Za-z0-9]{16,}",
        // JWTs
        r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}",
        // npm tokens
        r"\bnpm_[A-Za-z0-9]{36}",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid regex"))
    .collect()
});

/// `KEY=value` / `"password": "value"` style assignments: only the value is masked.
static ASSIGNMENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(?P<key>[A-Za-z0-9_\-\.]*(?:api[_\-]?key|secret|passw(?:or)?d|pwd|token|access[_\-]?key|private[_\-]?key|client[_\-]?secret|auth)[A-Za-z0-9_\-\.]*)(?P<sep>["']?\s*[:=]\s*["']?)(?P<val>[^\s"',;]{6,})"#,
    )
    .expect("valid regex")
});

/// `Authorization: Bearer xxx`
static BEARER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?P<pre>\bbearer\s+)(?P<val>[A-Za-z0-9_\-\.=+/]{12,})").expect("valid regex"));

/// Credentials embedded in URLs: `scheme://user:password@host`
static URL_CREDS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?P<pre>[a-zA-Z][a-zA-Z0-9+\-.]*://[^\s:/@]+:)(?P<val>[^\s@/]+)(?P<at>@)").expect("valid regex"));

pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for re in WHOLE.iter() {
        out = re.replace_all(&out, MASK).into_owned();
    }
    out = URL_CREDS
        .replace_all(&out, |c: &Captures| format!("{}{}{}", &c["pre"], MASK, &c["at"]))
        .into_owned();
    out = BEARER
        .replace_all(&out, |c: &Captures| format!("{}{}", &c["pre"], MASK))
        .into_owned();
    out = ASSIGNMENT
        .replace_all(&out, |c: &Captures| {
            let val = &c["val"];
            // Leave obvious non-secrets alone (already masked, env refs, placeholders).
            if val.contains(MASK) || val.eq_ignore_ascii_case("bearer") || val.eq_ignore_ascii_case("basic") || val.starts_with('$') || val.starts_with("process.env") || val.starts_with("os.environ") || val.starts_with('<') {
                c[0].to_string()
            } else {
                format!("{}{}{}", &c["key"], &c["sep"], MASK)
            }
        })
        .into_owned();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_known_key_formats() {
        let cases = [
            "use sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "AKIAIOSFODNN7EXAMPLE",
            "AIzaSyA1234567890abcdefghijklmnopqrstuv",
            "xoxb-1234567890-abcdefghij",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
        ];
        for c in cases {
            let r = redact(c);
            assert!(r.contains(MASK), "not masked: {c} -> {r}");
        }
    }

    #[test]
    fn masks_assignments_but_keeps_key_names() {
        assert_eq!(redact("DB_PASSWORD=hunter2hunter2"), "DB_PASSWORD=[REDACTED]");
        assert_eq!(redact(r#""apiKey": "abc123def456""#), r#""apiKey": "[REDACTED]""#);
        assert_eq!(redact("OPENAI_API_KEY = 'zzzzzzzzzzzz'"), "OPENAI_API_KEY = '[REDACTED]'");
        assert_eq!(redact("Authorization: Bearer abcdefghijklmnop"), "Authorization: Bearer [REDACTED]");
        assert_eq!(
            redact("postgres://admin:s3cretpw@db.local:5432/app"),
            "postgres://admin:[REDACTED]@db.local:5432/app"
        );
    }

    #[test]
    fn leaves_env_references_and_plain_text() {
        assert_eq!(redact("API_KEY=$API_KEY"), "API_KEY=$API_KEY");
        assert_eq!(redact("const token = process.env.TOKEN"), "const token = process.env.TOKEN");
        let plain = "Please refactor the auth module and add tests.";
        assert_eq!(redact(plain), plain);
        assert_eq!(redact("https://example.com/path"), "https://example.com/path");
    }

    #[test]
    fn masks_private_keys() {
        let pem = "x\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\ndef\n-----END RSA PRIVATE KEY-----\ny";
        assert_eq!(redact(pem), "x\n[REDACTED]\ny");
    }
}
