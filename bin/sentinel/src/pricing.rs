use std::collections::HashMap;

use once_cell::sync::Lazy;

/// Per-million-token prices in USD. Captured here rather than fetched
/// remotely so cost stays deterministic and the sidecar has no
/// dependency on a public price API.
#[derive(Debug, Clone, Copy)]
pub struct ModelPricing {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_creation_per_mtok: f64,
}

pub static PRICING: Lazy<HashMap<&'static str, ModelPricing>> = Lazy::new(|| {
    let mut m = HashMap::new();
    // Anthropic — illustrative; refresh from official pricing as needed.
    m.insert(
        "claude-opus-4-7",
        ModelPricing {
            input_per_mtok: 15.0,
            output_per_mtok: 75.0,
            cache_read_per_mtok: 1.5,
            cache_creation_per_mtok: 18.75,
        },
    );
    m.insert(
        "claude-sonnet-4-6",
        ModelPricing {
            input_per_mtok: 3.0,
            output_per_mtok: 15.0,
            cache_read_per_mtok: 0.3,
            cache_creation_per_mtok: 3.75,
        },
    );
    m.insert(
        "claude-haiku-4-5",
        ModelPricing {
            input_per_mtok: 0.8,
            output_per_mtok: 4.0,
            cache_read_per_mtok: 0.08,
            cache_creation_per_mtok: 1.0,
        },
    );
    // OpenAI — illustrative defaults.
    m.insert(
        "gpt-4o",
        ModelPricing {
            input_per_mtok: 2.5,
            output_per_mtok: 10.0,
            cache_read_per_mtok: 1.25,
            cache_creation_per_mtok: 2.5,
        },
    );
    m.insert(
        "gpt-4o-mini",
        ModelPricing {
            input_per_mtok: 0.15,
            output_per_mtok: 0.6,
            cache_read_per_mtok: 0.075,
            cache_creation_per_mtok: 0.15,
        },
    );
    m
});

/// Compute the cost in USD given a token breakdown. Falls back to a
/// conservative `claude-sonnet-4-6` price when the model is unknown so
/// the cost meter doesn't silently report $0 for new releases.
pub fn cost_usd(
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
) -> f64 {
    let p = PRICING.get(model).copied().unwrap_or_else(|| {
        *PRICING
            .get("claude-sonnet-4-6")
            .expect("default pricing missing")
    });
    let mtok = 1_000_000.0;
    (input_tokens as f64 / mtok) * p.input_per_mtok
        + (output_tokens as f64 / mtok) * p.output_per_mtok
        + (cache_read_tokens as f64 / mtok) * p.cache_read_per_mtok
        + (cache_creation_tokens as f64 / mtok) * p.cache_creation_per_mtok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_priced_exactly() {
        // 1M input + 1M output on Opus-4-7 = 15 + 75 = 90 USD
        let c = cost_usd("claude-opus-4-7", 1_000_000, 1_000_000, 0, 0);
        assert!((c - 90.0).abs() < 1e-9, "{c}");
    }

    #[test]
    fn unknown_model_falls_back() {
        let c = cost_usd("never-heard-of-it", 1_000, 1_000, 0, 0);
        assert!(c > 0.0);
    }
}
