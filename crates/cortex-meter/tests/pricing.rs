//! Estimates from the price table: the shipped table checked against Anthropic's pricing page as
//! it read on 2026-11-03, costs worked out by hand, every reason a response has no estimate, the
//! files that are not price tables, and the user's file written once and then read as it is.

use cortex_meter::{
    Billed, Cost, MAX_PRICE_FILE_BYTES, ModelPrices, PriceTable, PricingError, Rates,
    SHIPPED_PRICES, Unpriced, load, load_or_write,
};

/// Result type for these tests: any failure fails the test.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Rates in picodollars per token from prices in dollars per million tokens times a million:
/// input, five-minute and one-hour cache writes, cache reads, output.
fn rates([input, cache_write_5m, cache_write_1h, cache_read, output]: [u64; 5]) -> Rates {
    Rates {
        input,
        cache_write_5m,
        cache_write_1h,
        cache_read,
        output,
    }
}

/// A response of `model` served at the standard tier and speed, run anywhere, that took `input`
/// tokens of uncached input and nothing else.
fn billed(model: &str, input: u64) -> Billed<'_> {
    Billed {
        model,
        input,
        cache_write: 0,
        cache_write_1h: Some(0),
        cache_read: 0,
        output: 0,
        service_tier: Some("standard"),
        speed: Some("standard"),
        inference_geo: Some("global"),
    }
}

/// Each model of the shipped table as the pricing page listed it on 2026-11-03, prices in
/// picodollars per token (dollars per million tokens times a million, worked out by hand): its
/// id; input, five-minute and one-hour cache writes, cache reads, output; its fast mode prices
/// (the page's fast input and output, with the cache multipliers on top of them: 1.25x and 2x for
/// writes, the model's own for hits); whether it charges 1.1x for US-only inference (the 4.6
/// generation and later); and Haiku 5.5's prices over 100,000 prompt tokens.
type PageRow = (
    &'static str,
    [u64; 5],
    Option<[u64; 5]>,
    bool,
    Option<(u64, [u64; 5])>,
);

/// The page's rows, copied by hand.
const PAGE: [PageRow; 14] = [
    (
        "claude-fable-5-1",
        [10_000_000, 12_500_000, 20_000_000, 250_000, 50_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-fable-5",
        [10_000_000, 12_500_000, 20_000_000, 1_000_000, 50_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-opus-5-5",
        [4_000_000, 5_000_000, 8_000_000, 200_000, 20_000_000],
        Some([8_000_000, 10_000_000, 16_000_000, 400_000, 40_000_000]),
        true,
        None,
    ),
    (
        "claude-opus-5",
        [5_000_000, 6_250_000, 10_000_000, 500_000, 25_000_000],
        Some([10_000_000, 12_500_000, 20_000_000, 1_000_000, 50_000_000]),
        true,
        None,
    ),
    (
        "claude-opus-4-8",
        [5_000_000, 6_250_000, 10_000_000, 500_000, 25_000_000],
        Some([10_000_000, 12_500_000, 20_000_000, 1_000_000, 50_000_000]),
        true,
        None,
    ),
    (
        "claude-opus-4-7",
        [5_000_000, 6_250_000, 10_000_000, 500_000, 25_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-opus-4-6",
        [5_000_000, 6_250_000, 10_000_000, 500_000, 25_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-opus-4-5",
        [5_000_000, 6_250_000, 10_000_000, 500_000, 25_000_000],
        None,
        false,
        None,
    ),
    (
        "claude-sonnet-5-5",
        [2_000_000, 2_500_000, 4_000_000, 100_000, 10_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-sonnet-5",
        [2_000_000, 2_500_000, 4_000_000, 200_000, 10_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-sonnet-4-6",
        [3_000_000, 3_750_000, 6_000_000, 300_000, 15_000_000],
        None,
        true,
        None,
    ),
    (
        "claude-sonnet-4-5",
        [3_000_000, 3_750_000, 6_000_000, 300_000, 15_000_000],
        None,
        false,
        None,
    ),
    (
        "claude-haiku-5-5",
        [100_000, 125_000, 200_000, 10_000, 500_000],
        None,
        true,
        Some((100_000, [500_000, 625_000, 1_000_000, 50_000, 2_500_000])),
    ),
    (
        "claude-haiku-4-5",
        [1_000_000, 1_250_000, 2_000_000, 100_000, 5_000_000],
        None,
        false,
        None,
    ),
];

/// The shipped table is the pricing page's, read on 2026-11-03, with its date and source: every
/// model the page lists without marking it retired, each with exactly the page's prices, and no
/// other. Mythos, whose ids the models pages do not give, is not one of them.
#[test]
fn the_shipped_table_is_the_pricing_page_s() -> TestResult {
    let table = PriceTable::parse(SHIPPED_PRICES)?;
    assert_eq!(table.updated(), "2026-11-03");
    assert_eq!(
        table.source(),
        Some("https://platform.claude.com/docs/en/about-claude/pricing")
    );
    for (id, standard, fast, is_us_priced, long_prompt) in PAGE {
        let prices = table.model(id).ok_or(id)?;
        let expected = ModelPrices {
            standard: rates(standard),
            fast: fast.map(rates),
            long_prompt: long_prompt.map(|(above, prices)| (above, rates(prices))),
            us_only_per_mille: is_us_priced.then_some(1_100),
        };
        assert_eq!(prices, &expected, "{id}");
    }
    assert_eq!(SHIPPED_PRICES.matches("\n[models.").count(), PAGE.len());
    assert!(table.model("claude-mythos-5-1").is_none());
    Ok(())
}

/// Costs worked out by hand, in picodollars (a dollar per million tokens is a million
/// picodollars per token):
///
/// - The Grep session's first response, Claude Sonnet 5.5: 3,702 input at 2,000,000, 29,797
///   written for an hour at 4,000,000 and 100 output at 10,000,000: 7,404,000,000 +
///   119,188,000,000 + 1,000,000,000 = 127,592,000,000, about $0.128.
/// - Claude Opus 5.5 in fast mode, kept in the United States: 1,000 input at 8,000,000, 2,000
///   written for five minutes at 10,000,000, 10,000 read at 400,000, 500 output at 40,000,000:
///   8,000,000,000 + 20,000,000,000 + 4,000,000,000 + 20,000,000,000 = 52,000,000,000, times 1.1:
///   57,200,000,000.
/// - Cache writes split 600 for five minutes and 400 for an hour, Claude Sonnet 5.5: 600 at
///   2,500,000 and 400 at 4,000,000: 3,100,000,000.
/// - Claude Haiku 5.5 with a prompt of exactly 100,000 tokens pays the lower price, 100,000 at
///   100,000: 10,000,000,000; with 100,001 tokens, the higher, at 500,000: 50,000,500,000.
/// - A dated id takes its undated prices: `claude-haiku-4-5-20251001`, 1,000 input at 1,000,000:
///   1,000,000,000.
#[test]
fn a_response_s_cost_is_worked_out_from_its_prices() -> TestResult {
    let table = PriceTable::parse(SHIPPED_PRICES)?;
    let estimate = |billed: Billed<'_>| table.estimate(&billed).map(Cost::picodollars);
    let grep_first = Billed {
        cache_write: 29_797,
        cache_write_1h: Some(29_797),
        output: 100,
        ..billed("claude-sonnet-5-5", 3_702)
    };
    assert_eq!(estimate(grep_first), Ok(127_592_000_000));
    let fast_in_the_us = Billed {
        cache_write: 2_000,
        cache_write_1h: Some(0),
        cache_read: 10_000,
        output: 500,
        speed: Some("fast"),
        inference_geo: Some("us"),
        ..billed("claude-opus-5-5", 1_000)
    };
    assert_eq!(estimate(fast_in_the_us), Ok(57_200_000_000));
    let split = Billed {
        cache_write: 1_000,
        cache_write_1h: Some(400),
        ..billed("claude-sonnet-5-5", 0)
    };
    assert_eq!(estimate(split), Ok(3_100_000_000));
    assert_eq!(
        estimate(billed("claude-haiku-5-5", 100_000)),
        Ok(10_000_000_000)
    );
    assert_eq!(
        estimate(billed("claude-haiku-5-5", 100_001)),
        Ok(50_000_500_000)
    );
    assert_eq!(
        estimate(billed("claude-haiku-4-5-20251001", 1_000)),
        Ok(1_000_000_000)
    );
    // A prompt counts its cache reads and writes too: 10 input, 5 written for five minutes and
    // 100,000 read make 100,015 tokens, over the threshold, at the higher prices: 10 at 500,000,
    // 5 at 625,000 and 100,000 at 50,000: 5,000,000 + 3,125,000 + 5,000,000,000 = 5,008,125,000;
    // kept in the United States, times 1.1: 5,508,937,500.
    let cached = Billed {
        cache_write: 5,
        cache_write_1h: Some(0),
        cache_read: 100_000,
        ..billed("claude-haiku-5-5", 10)
    };
    assert_eq!(estimate(cached), Ok(5_008_125_000));
    let cached_in_the_us = Billed {
        inference_geo: Some("us"),
        ..cached
    };
    assert_eq!(estimate(cached_in_the_us), Ok(5_508_937_500));
    // The two first costs together: 127,592,000,000 + 57,200,000,000.
    let together = table
        .estimate(&grep_first)
        .and_then(|cost| Ok(cost.plus(table.estimate(&fast_in_the_us)?)));
    assert_eq!(together.map(Cost::picodollars), Ok(184_792_000_000));
    Ok(())
}

/// A response missing anything its price depends on has no estimate, and says which: a model not
/// in the table, or an id whose last part is no date; a tier other than standard, or none; a
/// speed the model has no prices for, or none; a region the model has no price for, or none;
/// cache writes with no split, or a split larger than the writes. A response that wrote nothing
/// to the cache needs no split.
#[test]
fn a_response_missing_what_its_price_depends_on_has_no_estimate() -> TestResult {
    let table = PriceTable::parse(SHIPPED_PRICES)?;
    let sonnet = billed("claude-sonnet-5-5", 10);
    let cases = [
        (billed("claude-mythos-5-1", 10), Err(Unpriced::Model)),
        (billed("claude-opus-5-5-preview", 10), Err(Unpriced::Model)),
        (billed("claude-opus-5-5-2026110", 10), Err(Unpriced::Model)),
        (
            Billed {
                service_tier: Some("batch"),
                ..sonnet
            },
            Err(Unpriced::Tier),
        ),
        (
            Billed {
                service_tier: None,
                ..sonnet
            },
            Err(Unpriced::Tier),
        ),
        (
            Billed {
                speed: Some("fast"),
                ..sonnet
            },
            Err(Unpriced::Speed),
        ),
        (
            Billed {
                speed: None,
                ..sonnet
            },
            Err(Unpriced::Speed),
        ),
        (
            Billed {
                speed: Some("fast"),
                ..billed("claude-haiku-5-5", 10)
            },
            Err(Unpriced::Speed),
        ),
        (
            Billed {
                inference_geo: Some("us"),
                ..billed("claude-haiku-4-5", 10)
            },
            Err(Unpriced::Region),
        ),
        (
            Billed {
                inference_geo: Some("eu"),
                ..sonnet
            },
            Err(Unpriced::Region),
        ),
        (
            Billed {
                inference_geo: None,
                ..sonnet
            },
            Err(Unpriced::Region),
        ),
        (
            Billed {
                cache_write: 5,
                cache_write_1h: None,
                ..sonnet
            },
            Err(Unpriced::CacheSplit),
        ),
        (
            Billed {
                cache_write: 5,
                cache_write_1h: Some(6),
                ..sonnet
            },
            Err(Unpriced::CacheSplit),
        ),
        (
            Billed {
                cache_write_1h: None,
                ..sonnet
            },
            Ok(20_000_000),
        ),
    ];
    for (response, expected) in cases {
        assert_eq!(
            table.estimate(&response).map(Cost::picodollars),
            expected,
            "{response:?}"
        );
    }
    Ok(())
}

/// The shipped table with `old` replaced by `new`, the replacement checked to happen once.
fn edited(old: &str, new: &str) -> TestResult<String> {
    assert_eq!(SHIPPED_PRICES.matches(old).count(), 1, "{old}");
    Ok(SHIPPED_PRICES.replace(old, new))
}

/// A table of one model, `m`, whose lines are `lines`.
fn one_model(lines: &str) -> String {
    format!("updated = \"2026-11-03\"\n\n[models.m]\n{lines}\n")
}

/// Every price 1.0, in a model's table of prices.
const ONES: &str =
    "{ input = 1.0, cache_write_5m = 1.0, cache_write_1h = 1.0, cache_read = 1.0, output = 1.0 }";

/// What a file that is not a price table is refused for: a key the table does not know, a
/// negative price, one that is not a number, one too large, a date that is not `YYYY-MM-DD`,
/// long-prompt prices with no prompt size, a region multiplier of zero or too small to keep, a price
/// above zero too small to keep, a price the table does not know, and dates that are no day (a
/// thirteenth month, the 29th of February 2026). A table of one model with every price 1.0 and
/// nothing else is a table, and so is one with a price of zero, or dated 29 February 2028.
#[test]
fn a_file_that_is_not_a_price_table_is_refused() -> TestResult {
    let standard = |prices: &str| format!("standard = {prices}");
    let refusals = [
        (edited("updated =", "updatd =")?, "syntax"),
        (
            one_model(&standard(&ONES.replace("input = 1.0", "input = -1.0"))),
            "price",
        ),
        (
            one_model(&standard(&ONES.replace("input = 1.0", "input = nan"))),
            "price",
        ),
        (
            one_model(&standard(&ONES.replace("input = 1.0", "input = 2e6"))),
            "price",
        ),
        (
            edited("updated = \"2026-11-03\"", "updated = \"3 November 2026\"")?,
            "date",
        ),
        (
            edited("long_prompt_above_tokens = 100000", "")?,
            "long prompt",
        ),
        (
            one_model(&format!("{}\nus_only = 0.0", standard(ONES))),
            "price",
        ),
        (
            one_model(&standard(
                &ONES.replace("output = 1.0", "output = 1.0, outputs = 1.0"),
            )),
            "syntax",
        ),
        (
            one_model(&format!("{}\nus_only = 0.0004", standard(ONES))),
            "price",
        ),
        (
            one_model(&standard(&ONES.replace("input = 1.0", "input = 0.0000004"))),
            "price",
        ),
        (
            one_model(&standard(ONES)).replace("2026-11-03", "2026-13-01"),
            "date",
        ),
        (
            one_model(&standard(ONES)).replace("2026-11-03", "2026-02-29"),
            "date",
        ),
        (one_model(&standard(ONES)), "a table"),
        (
            one_model(&standard(&ONES.replace("input = 1.0", "input = 0.0"))),
            "a table",
        ),
        (
            one_model(&standard(ONES)).replace("2026-11-03", "2028-02-29"),
            "a table",
        ),
    ];
    for (text, expected) in refusals {
        let found = match PriceTable::parse(&text) {
            Err(PricingError::Syntax(_)) => "syntax",
            Err(PricingError::Price { .. }) => "price",
            Err(PricingError::Date) => "date",
            Err(PricingError::LongPrompt { .. }) => "long prompt",
            Err(_) => "another error",
            Ok(_) => "a table",
        };
        assert_eq!(found, expected, "{text}");
    }
    Ok(())
}

/// The first time a price table is needed, the shipped one is written to the user's file, byte
/// for byte, and nothing else is left in its folder; once the user has edited it, their prices are
/// read and the file is left as it is, a whole number accepted as a price. A file in a folder that
/// is not there is not written; a file larger than a price table is refused as one, whatever
/// characters it holds, and one that is not UTF-8 text as not a table.
#[test]
fn the_shipped_table_is_written_once_and_the_user_s_prices_win() -> TestResult {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("pricing.toml");
    let first = load_or_write(&path)?;
    assert_eq!(std::fs::read_to_string(&path)?, SHIPPED_PRICES);
    assert_eq!(first, PriceTable::parse(SHIPPED_PRICES)?);
    let names: Vec<String> = std::fs::read_dir(folder.path())?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<_>>()?;
    assert_eq!(names, ["pricing.toml"]);
    // Claude Sonnet 5.5's standard prices, the only line that gives these five.
    let edited = edited(
        "{ input = 2.0, cache_write_5m = 2.5, cache_write_1h = 4.0, cache_read = 0.1, output = 10.0 }",
        "{ input = 3, cache_write_5m = 2.5, cache_write_1h = 4.0, cache_read = 0.1, output = 10.0 }",
    )?;
    std::fs::write(&path, &edited)?;
    let theirs = load_or_write(&path)?;
    assert_eq!(
        theirs
            .model("claude-sonnet-5-5")
            .map(|prices| prices.standard.input),
        Some(3_000_000)
    );
    assert_eq!(std::fs::read_to_string(&path)?, edited);
    let nowhere = folder.path().join("missing").join("pricing.toml");
    assert!(matches!(
        load_or_write(&nowhere),
        Err(PricingError::Io { .. })
    ));
    assert!(!nowhere.exists());
    let large = folder.path().join("large.toml");
    // Two-byte characters, so that the cut past the limit falls inside one.
    let accents = "\u{e9}".repeat(usize::try_from(MAX_PRICE_FILE_BYTES)? / 2 + 1);
    std::fs::write(&large, accents)?;
    assert!(matches!(
        load_or_write(&large),
        Err(PricingError::TooLarge(_))
    ));
    let latin = folder.path().join("latin.toml");
    std::fs::write(&latin, b"updated = \"2026-11-03\" # caf\xe9\n")?;
    assert!(matches!(
        load_or_write(&latin),
        Err(PricingError::Syntax(_))
    ));
    Ok(())
}

/// The read-only loader writes nothing: no file gives no table and leaves the folder empty, and a
/// file that is there is read as the writing loader reads it.
#[test]
fn the_read_only_loader_never_writes() -> TestResult {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("pricing.toml");
    assert_eq!(load(&path)?, None);
    assert_eq!(std::fs::read_dir(folder.path())?.count(), 0);
    std::fs::write(&path, SHIPPED_PRICES)?;
    assert_eq!(load(&path)?, Some(PriceTable::parse(SHIPPED_PRICES)?));
    Ok(())
}

/// Unix only, where a file has a mode: the price file written is the user's alone.
#[cfg(unix)]
#[test]
fn the_price_file_written_is_private() -> TestResult {
    use std::os::unix::fs::PermissionsExt;
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("pricing.toml");
    load_or_write(&path)?;
    assert_eq!(
        std::fs::metadata(&path)?.permissions().mode() & 0o777,
        0o600
    );
    Ok(())
}
