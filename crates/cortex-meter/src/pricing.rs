//! What model responses cost, estimated from a table of prices: the one Cortex ships, read from
//! Anthropic's pricing page on the date it carries, or the user's own copy of it.
//!
//! A file gives prices in US dollars per million tokens; here they are whole picodollars per
//! token (a dollar per million tokens is a million picodollars per token), so that estimates add
//! up exactly and the same rows always give the same estimate. A response is priced only when the
//! table knows everything its price depends on: its model, service tier, speed, where it ran, and
//! how its cache writes split between the two lifetimes. Missing any of them, it has no estimate,
//! never a guessed one.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Deserialize;

/// The price table Cortex ships, written to the user's folder the first time one is needed.
pub const SHIPPED_PRICES: &str = include_str!("../pricing.toml");

/// The highest price a file may give, in dollars per million tokens: far above any model's, and
/// low enough that no count of tokens can make an estimate overflow.
const MAX_DOLLARS_PER_MILLION: f64 = 1_000_000.0;

/// Picodollars per token in a dollar per million tokens.
const PICODOLLARS_PER_DOLLAR_PER_MILLION: f64 = 1_000_000.0;

/// The largest multiplier a file may give for inference kept in one region.
const MAX_REGION_MULTIPLIER: f64 = 10.0;

/// A multiplier is kept in thousandths, so that 1.1 is exactly 1,100.
const PER_MILLE: u128 = 1_000;

/// The prices of one way of serving a model, in picodollars per token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rates {
    /// Input read at the full price.
    pub input: u64,
    /// Input written to the five-minute cache.
    pub cache_write_5m: u64,
    /// Input written to the one-hour cache.
    pub cache_write_1h: u64,
    /// Input read from the cache.
    pub cache_read: u64,
    /// Output, reasoning included.
    pub output: u64,
}

/// One model's prices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPrices {
    /// At the standard speed.
    pub standard: Rates,
    /// In fast mode, where the model has one.
    pub fast: Option<Rates>,
    /// For a prompt over the number of tokens given, at the standard speed, where the model
    /// prices long prompts apart.
    pub long_prompt: Option<(u64, Rates)>,
    /// The multiplier, in thousandths, for inference kept in the United States, where the model
    /// offers it.
    pub us_only_per_mille: Option<u32>,
}

/// A table of prices and the date its prices were read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceTable {
    /// When the prices were read, as `YYYY-MM-DD`.
    updated: String,
    /// Where they were read from, when the file says.
    source: Option<String>,
    /// Each model's prices, by model id.
    models: BTreeMap<String, ModelPrices>,
}

/// What a response's price depends on: its model, its counts and how it was served.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Billed<'a> {
    /// The model's id.
    pub model: &'a str,
    /// Input read at the full price.
    pub input: u64,
    /// Input written to the cache, at either lifetime.
    pub cache_write: u64,
    /// Of `cache_write`, what was written for an hour, when known.
    pub cache_write_1h: Option<u64>,
    /// Input read from the cache.
    pub cache_read: u64,
    /// Output.
    pub output: u64,
    /// The service tier, when known.
    pub service_tier: Option<&'a str>,
    /// The speed, when known.
    pub speed: Option<&'a str>,
    /// Where it ran, when known.
    pub inference_geo: Option<&'a str>,
}

/// An estimated cost, in picodollars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Cost(u128);

impl Cost {
    /// The cost in picodollars, a millionth of a millionth of a dollar.
    #[must_use]
    pub fn picodollars(self) -> u128 {
        self.0
    }

    /// This cost and `other` together.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }
}

/// Why a response has no estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unpriced {
    /// Its model is not in the table.
    Model,
    /// It was served at a tier other than standard, or the transcript did not say.
    Tier,
    /// Its speed has no prices in the table, or the transcript did not say it.
    Speed,
    /// Where it ran has no price in the table, or the transcript did not say.
    Region,
    /// It wrote to the cache and the transcript did not say for how long.
    CacheSplit,
}

/// A price table that could not be had.
#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    /// The file is not a price table: TOML it cannot read, a key it does not know, or a field
    /// missing.
    #[error("the price file is not a price table: {0}")]
    Syntax(String),
    /// A price that is negative, not a number, or too large.
    #[error("the price file gives {model} a {field} that is not a price")]
    Price {
        /// The model.
        model: String,
        /// The price's name.
        field: &'static str,
    },
    /// A long-prompt price without the prompt size it starts above, or the size alone.
    #[error("the price file gives {model} long-prompt prices and no prompt size, or the reverse")]
    LongPrompt {
        /// The model.
        model: String,
    },
    /// The date is not a day written `YYYY-MM-DD`.
    #[error("the price file's date is not a day written YYYY-MM-DD")]
    Date,
    /// The file holds more than [`crate::MAX_PRICE_FILE_BYTES`].
    #[error("{} is larger than a price table", .0.display())]
    TooLarge(PathBuf),
    /// The file could not be read, written or tidied up after.
    #[error("{action} {}: {source}", path.display())]
    Io {
        /// Reading, writing or removing.
        action: &'static str,
        /// The file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
}

impl PriceTable {
    /// The table `text` holds, a price file's TOML.
    ///
    /// # Errors
    ///
    /// [`PricingError::Syntax`], [`PricingError::Price`], [`PricingError::LongPrompt`] and
    /// [`PricingError::Date`].
    pub fn parse(text: &str) -> Result<Self, PricingError> {
        let file: PriceFile =
            toml::from_str(text).map_err(|error| PricingError::Syntax(error.to_string()))?;
        if !is_day(&file.updated) {
            return Err(PricingError::Date);
        }
        let models = file
            .models
            .into_iter()
            .map(|(model, prices)| {
                let prices = prices.checked(&model)?;
                Ok((model, prices))
            })
            .collect::<Result<_, PricingError>>()?;
        Ok(Self {
            updated: file.updated,
            source: file.source,
            models,
        })
    }

    /// When the prices were read, as `YYYY-MM-DD`.
    #[must_use]
    pub fn updated(&self) -> &str {
        &self.updated
    }

    /// Where the prices were read from, when the file says.
    #[must_use]
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// The prices of the model `id`: its own, or for a dated id such as
    /// `claude-haiku-4-5-20251001`, those of the id without its date.
    #[must_use]
    pub fn model(&self, id: &str) -> Option<&ModelPrices> {
        if let Some(prices) = self.models.get(id) {
            return Some(prices);
        }
        let (undated, date) = id.rsplit_once('-')?;
        let is_date = date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit());
        is_date.then(|| self.models.get(undated)).flatten()
    }

    /// The estimated cost of the response `billed`, or why it has none.
    ///
    /// # Errors
    ///
    /// The [`Unpriced`] reason, the first of its model, tier, region, cache split and speed that
    /// the table or the transcript leaves unknown.
    pub fn estimate(&self, billed: &Billed<'_>) -> Result<Cost, Unpriced> {
        let prices = self.model(billed.model).ok_or(Unpriced::Model)?;
        if billed.service_tier != Some("standard") {
            return Err(Unpriced::Tier);
        }
        let per_mille = match billed.inference_geo {
            Some("global") => PER_MILLE,
            Some("us") => u128::from(prices.us_only_per_mille.ok_or(Unpriced::Region)?),
            _ => return Err(Unpriced::Region),
        };
        let one_hour = match billed.cache_write_1h {
            Some(one_hour) if one_hour <= billed.cache_write => one_hour,
            None if billed.cache_write == 0 => 0,
            _ => return Err(Unpriced::CacheSplit),
        };
        let rates = match billed.speed {
            Some("fast") => prices.fast.ok_or(Unpriced::Speed)?,
            Some("standard") => prices.at_standard_speed(billed),
            _ => return Err(Unpriced::Speed),
        };
        let cost = |tokens: u64, rate: u64| u128::from(tokens) * u128::from(rate);
        let picodollars = cost(billed.input, rates.input)
            + cost(billed.cache_write - one_hour, rates.cache_write_5m)
            + cost(one_hour, rates.cache_write_1h)
            + cost(billed.cache_read, rates.cache_read)
            + cost(billed.output, rates.output);
        // Rounded down to a whole picodollar, which the shipped prices never need.
        Ok(Cost(picodollars * per_mille / PER_MILLE))
    }
}

/// Whether `text` is a day of the Gregorian calendar written `YYYY-MM-DD`.
fn is_day(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let is_digits =
        |part: &str, length| part.len() == length && part.bytes().all(|byte| byte.is_ascii_digit());
    if !(is_digits(year, 4) && is_digits(month, 2) && is_digits(day, 2)) {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<u32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let is_leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day)
}

impl ModelPrices {
    /// The rates of the standard speed for `billed`: the long-prompt rates when its prompt, its
    /// input, cache writes and cache reads together, is over the size they start above.
    fn at_standard_speed(&self, billed: &Billed<'_>) -> Rates {
        let prompt = billed
            .input
            .saturating_add(billed.cache_write)
            .saturating_add(billed.cache_read);
        match self.long_prompt {
            Some((above, rates)) if prompt > above => rates,
            _ => self.standard,
        }
    }
}

/// A price file as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PriceFile {
    /// When its prices were read.
    updated: String,
    /// Where they were read from.
    source: Option<String>,
    /// Each model's prices.
    models: BTreeMap<String, ModelFile>,
}

/// One model's prices as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelFile {
    /// At the standard speed.
    standard: RatesFile,
    /// In fast mode.
    fast: Option<RatesFile>,
    /// For a long prompt.
    long_prompt: Option<RatesFile>,
    /// The prompt size long-prompt prices start above.
    long_prompt_above_tokens: Option<u64>,
    /// The multiplier for inference kept in the United States.
    us_only: Option<f64>,
}

/// One way of serving a model's prices as written, in dollars per million tokens.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RatesFile {
    /// Input.
    input: f64,
    /// Five-minute cache writes.
    cache_write_5m: f64,
    /// One-hour cache writes.
    cache_write_1h: f64,
    /// Cache reads.
    cache_read: f64,
    /// Output.
    output: f64,
}

impl ModelFile {
    /// The prices of `model`, each checked.
    fn checked(self, model: &str) -> Result<ModelPrices, PricingError> {
        let long_prompt = match (self.long_prompt, self.long_prompt_above_tokens) {
            (Some(rates), Some(above)) => Some((above, rates.checked(model)?)),
            (None, None) => None,
            _ => {
                return Err(PricingError::LongPrompt {
                    model: model.to_owned(),
                });
            }
        };
        let us_only_per_mille = self
            .us_only
            .map(|multiplier| {
                let is_multiplier = multiplier.is_finite() && multiplier <= MAX_REGION_MULTIPLIER;
                // Zero, below zero, or too small to keep in thousandths, a multiplier would make
                // every such response free, which no price table means: each keeps as zero.
                let kept = per_mille(multiplier);
                if !is_multiplier || kept == 0 {
                    return Err(PricingError::Price {
                        model: model.to_owned(),
                        field: "us_only",
                    });
                }
                Ok(kept)
            })
            .transpose()?;
        Ok(ModelPrices {
            standard: self.standard.checked(model)?,
            fast: self.fast.map(|rates| rates.checked(model)).transpose()?,
            long_prompt,
            us_only_per_mille,
        })
    }
}

impl RatesFile {
    /// The rates in picodollars per token, each checked.
    fn checked(self, model: &str) -> Result<Rates, PricingError> {
        let rate = |field, dollars: f64| {
            let is_price =
                dollars.is_finite() && (0.0..=MAX_DOLLARS_PER_MILLION).contains(&dollars);
            // A price above zero too small to keep in picodollars would read as free; zero
            // itself is the user's to give.
            let kept = picodollars(dollars);
            if !is_price || (dollars > 0.0 && kept == 0) {
                return Err(PricingError::Price {
                    model: model.to_owned(),
                    field,
                });
            }
            Ok(kept)
        };
        Ok(Rates {
            input: rate("input", self.input)?,
            cache_write_5m: rate("cache_write_5m", self.cache_write_5m)?,
            cache_write_1h: rate("cache_write_1h", self.cache_write_1h)?,
            cache_read: rate("cache_read", self.cache_read)?,
            output: rate("output", self.output)?,
        })
    }
}

/// `dollars` per million tokens in whole picodollars per token, a price checked to lie between
/// zero and [`MAX_DOLLARS_PER_MILLION`].
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a checked price times a million is a whole number of at most 10^12, which a u64 holds"
)]
fn picodollars(dollars: f64) -> u64 {
    (dollars * PICODOLLARS_PER_DOLLAR_PER_MILLION).round() as u64
}

/// `multiplier` in thousandths, a finite multiplier at most [`MAX_REGION_MULTIPLIER`]; zero for
/// one below a two-thousandth, zero or below.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a checked multiplier times a thousand is a whole number of at most 10,000"
)]
fn per_mille(multiplier: f64) -> u32 {
    (multiplier * 1_000.0).round() as u32
}
