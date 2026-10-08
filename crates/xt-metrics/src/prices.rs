use crate::{Error, Result};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const COST_BASIS: &str = "Global public token API-equivalent, using recorded service tier, or OpenAI's default tier for Codex responses that record none; excludes codex-auto-review responses, unrecorded regional/speed modifiers, tool fees, and subscriptions.";

/// Validated immutable in-memory catalog. No external path or network loader.
#[derive(Clone, Debug)]
pub struct PriceCatalog(Catalog);
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: String,
    as_of: String,
    basis: String,
    rate_unit: String,
    sources: Vec<String>,
    models: Vec<Model>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Model {
    pub id: String,
    pub aliases: Vec<String>,
    pub cache_write_mode: CacheWriteMode,
    pub bands: Vec<Band>,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CacheWriteMode {
    Flat,
    TtlSplit,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Band {
    /// Inclusive prompt-token upper bound; the final band must be unbounded.
    pub max_prompt_tokens: Option<u64>,
    pub tiers: Vec<Tier>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tier {
    pub names: Vec<String>,
    pub rates: Rates,
}
/// Integer nano-USD per token, not USD per million. Null rates stay unavailable.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Rates {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
}
fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}
impl PriceCatalog {
    pub fn bundled() -> Result<Self> {
        Self::from_json(include_str!("prices.json"))
    }
    pub fn from_json(json: &str) -> Result<Self> {
        let catalog: Catalog =
            serde_json::from_str(json).map_err(|_| Error::InvalidPriceCatalog)?;
        if !valid_name(&catalog.version)
            || catalog.basis != COST_BASIS
            || catalog.rate_unit != "nano_usd_per_token"
            || catalog.as_of.len() != 10
            || catalog.as_of.parse::<jiff::civil::Date>().is_err()
            || catalog.models.is_empty()
            || catalog.sources.is_empty()
        {
            return Err(Error::InvalidPriceCatalog);
        }
        if catalog.sources.iter().any(|s| {
            !s.starts_with("https://")
                || s[8..].split('/').next().is_none_or(|host| host.is_empty())
                || s.chars().any(char::is_whitespace)
        }) {
            return Err(Error::InvalidPriceCatalog);
        }
        let mut names = BTreeSet::new();
        for model in &catalog.models {
            for name in std::iter::once(&model.id).chain(&model.aliases) {
                if !valid_name(name) || !names.insert(name) {
                    return Err(Error::InvalidPriceCatalog);
                }
            }
            if model.bands.is_empty() || model.bands.last().unwrap().max_prompt_tokens.is_some() {
                return Err(Error::InvalidPriceCatalog);
            }
            let mut previous = None;
            for (i, band) in model.bands.iter().enumerate() {
                match band.max_prompt_tokens {
                    None if i + 1 != model.bands.len() => return Err(Error::InvalidPriceCatalog),
                    Some(max) if previous.is_some_and(|p| max <= p) => {
                        return Err(Error::InvalidPriceCatalog);
                    }
                    _ => {}
                }
                previous = band.max_prompt_tokens;
                if band.tiers.is_empty() {
                    return Err(Error::InvalidPriceCatalog);
                }
                let mut tiers = BTreeSet::new();
                for tier in &band.tiers {
                    if tier.names.is_empty()
                        || tier
                            .names
                            .iter()
                            .any(|n| !valid_name(n) || !tiers.insert(n))
                    {
                        return Err(Error::InvalidPriceCatalog);
                    }
                    let rates = &tier.rates;
                    if match model.cache_write_mode {
                        CacheWriteMode::Flat => {
                            rates.cache_write_5m.is_some() || rates.cache_write_1h.is_some()
                        }
                        CacheWriteMode::TtlSplit => rates.cache_write.is_some(),
                    } {
                        return Err(Error::InvalidPriceCatalog);
                    }
                }
            }
        }
        Ok(Self(catalog))
    }
    pub fn version(&self) -> &str {
        &self.0.version
    }
    pub fn as_of(&self) -> &str {
        &self.0.as_of
    }
    pub(crate) fn model(&self, name: &str) -> Option<&Model> {
        self.0
            .models
            .iter()
            .find(|m| m.id == name || m.aliases.iter().any(|a| a == name))
    }
}
