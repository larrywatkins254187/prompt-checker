//! Rough cost estimation from token usage, using Anthropic list prices (USD per 1M tokens).
//! On a Pro/Max subscription nothing is billed per token; the number is still useful as
//! a relative measure of how heavy a session was.

use crate::model::Usage;

struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
}

fn price_for(model: &str) -> Option<Price> {
    let m = model.to_ascii_lowercase();
    let p = |input: f64, output: f64, cache_read: Option<f64>| Price {
        input,
        output,
        cache_read: cache_read.unwrap_or(input * 0.1),
    };
    // Order matters: more specific prefixes first.
    let table: &[(&str, Price)] = &[
        ("claude-fable", p(10.0, 50.0, Some(0.25))),
        ("claude-mythos", p(10.0, 50.0, Some(0.25))),
        ("claude-opus-5-5", p(4.0, 20.0, Some(0.20))),
        ("claude-opus-5", p(5.0, 25.0, None)),
        ("claude-opus-4-8", p(5.0, 25.0, None)),
        ("claude-opus-4-7", p(5.0, 25.0, None)),
        ("claude-opus-4-6", p(5.0, 25.0, None)),
        ("claude-opus-4-5", p(5.0, 25.0, None)),
        ("claude-opus-4", p(15.0, 75.0, None)),
        ("claude-sonnet-5", p(2.0, 10.0, Some(0.20))),
        ("claude-sonnet-4", p(3.0, 15.0, None)),
        ("claude-3-7-sonnet", p(3.0, 15.0, None)),
        ("claude-haiku-5-5", p(0.10, 0.50, None)),
        ("claude-haiku-4-5", p(1.0, 5.0, None)),
        ("claude-3-5-haiku", p(0.8, 4.0, None)),
    ];
    table.iter().find(|(prefix, _)| m.starts_with(prefix)).map(|(_, pr)| Price {
        input: pr.input,
        output: pr.output,
        cache_read: pr.cache_read,
    })
}

/// Estimated USD cost, or `None` for models without a known price (e.g. non-Claude models).
pub fn estimate(model: &str, u: &Usage) -> Option<f64> {
    let p = price_for(model)?;
    let cache_write = p.input * 1.25;
    Some(
        (u.input_tokens as f64 * p.input
            + u.output_tokens as f64 * p.output
            + u.cache_read_tokens as f64 * p.cache_read
            + u.cache_creation_tokens as f64 * cache_write)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_known_models() {
        let u = Usage { input_tokens: 1_000_000, output_tokens: 1_000_000, cache_read_tokens: 0, cache_creation_tokens: 0 };
        assert_eq!(estimate("claude-opus-5-5", &u), Some(24.0));
        assert_eq!(estimate("claude-sonnet-5-5", &u), Some(12.0));
        assert_eq!(estimate("claude-opus-4-1-20250805", &u), Some(90.0));
        assert_eq!(estimate("gpt-5-codex", &u), None);
    }
}
