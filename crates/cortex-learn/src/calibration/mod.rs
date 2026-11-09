//! Calibration: fitting the confidence model the gate uses once there are enough labelled
//! injections, and deciding whether to keep it.
//!
//! The model is fitted on the latest [`CALIBRATION_WINDOW`] labelled injections of prompts, those
//! the gate kept back and those of held-out tasks among them, which are labelled from what the
//! agent did without Cortex's help. It is judged on data it did not see: the newest fifth of the
//! window is held out, since the model is always used on prompts that come later than the ones it
//! was fitted on, and the rest fits it. Its AUC on the held-out fifth is compared with the kept
//! model's AUC on the same fifth, and it is kept only if it ranks better than chance and at most
//! [`AUC_TOLERANCE`] worse than the kept one. A window without both labels on either side fits
//! nothing. Until a model is kept the gate keeps its cold-start rule, and so it does while the
//! newest model kept is one this build cannot use ([`ModelInUse::Unusable`]), which upkeep then
//! replaces at its next run.

mod auc;
mod fit;
mod model;

use cortex_store::{CalibrationAttempt, LabelledInjection, StoreError, WriteTx};

pub use self::auc::auc;
pub use self::fit::L2_PENALTY;
pub use self::model::{ConfidenceModel, MODEL_INPUTS, model_inputs};

use self::fit::{Sample, fit};

/// How many of the latest labelled injections a model is fitted and judged on.
pub const CALIBRATION_WINDOW: usize = 2_000;

/// How many labelled injections there must be before a model is fitted: before then the gate
/// keeps its cold-start rule.
pub const MIN_LABELLED: usize = 200;

/// One in this many of the window, the newest, is held out to judge a model on.
pub const HELD_OUT_EVERY: usize = 5;

/// How much lower than the kept model's a new model's AUC may be and the new one still be kept:
/// a small loss traded for a model fitted on later work.
pub const AUC_TOLERANCE: f64 = 0.01;

/// The AUC a model must beat to be kept at all: one that ranks no better than chance would only
/// replace the cold-start rule with noise.
pub const CHANCE_AUC: f64 = 0.5;

/// How long after one fitting the next may run, in milliseconds: a day, as heavy upkeep runs at
/// most once a day.
pub const REFIT_AFTER_MS: i64 = 24 * 60 * 60 * 1_000;

/// How many models are kept: the newest, which is in use, and those before it, as a record.
pub const KEPT_MODELS: u32 = 30;

/// A model fitted and kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fitted {
    /// The model.
    pub model: ConfidenceModel,
    /// How many labelled injections it was fitted on.
    pub samples: u32,
    /// Its AUC on the held-out injections.
    pub auc: f64,
}

/// What became of one fitting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Calibrated {
    /// The new model is kept and in use from now on.
    Kept(Fitted),
    /// No model was kept: why.
    Refused(Refusal),
}

/// Why a fitting kept no model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Refusal {
    /// Fewer than [`MIN_LABELLED`] labelled injections: how many there are.
    TooFew {
        /// How many there are.
        labelled: usize,
    },
    /// The injections fitted on or those held out are all used or all unused, so a model cannot
    /// be fitted or judged.
    OneSided,
    /// The fit gave numbers that are not finite.
    NotFitted,
    /// The new model ranks the held-out injections no better than chance.
    NoBetterThanChance {
        /// Its AUC.
        auc: f64,
    },
    /// The new model ranks the held-out injections worse than the kept one by more than
    /// [`AUC_TOLERANCE`].
    WorseThanKept {
        /// Its AUC.
        auc: f64,
        /// The kept model's AUC on the same injections.
        kept_auc: f64,
    },
}

/// Fits a model on `window`, labelled injections oldest first, and decides whether it replaces
/// `kept`, the model in use, as the module says.
#[must_use]
pub fn calibrated(window: &[LabelledInjection], kept: Option<&ConfidenceModel>) -> Calibrated {
    if window.len() < MIN_LABELLED {
        return Calibrated::Refused(Refusal::TooFew {
            labelled: window.len(),
        });
    }
    let held_out = window.len() / HELD_OUT_EVERY;
    let (fitting, judging) = window.split_at(window.len() - held_out);
    let samples: Vec<Sample> = fitting
        .iter()
        .map(|labelled| (model_inputs(&labelled.features), labelled.label))
        .collect();
    let has_both = |part: &[LabelledInjection]| {
        part.iter().any(|labelled| labelled.label) && part.iter().any(|labelled| !labelled.label)
    };
    if !has_both(fitting) || !has_both(judging) {
        return Calibrated::Refused(Refusal::OneSided);
    }
    let Some(model) = fit(&samples, L2_PENALTY) else {
        return Calibrated::Refused(Refusal::NotFitted);
    };
    let auc_of = |model: &ConfidenceModel| {
        let scored: Vec<(f64, bool)> = judging
            .iter()
            .map(|labelled| (model.probability(&labelled.features), labelled.label))
            .collect();
        auc(&scored)
    };
    let Some(new_auc) = auc_of(&model) else {
        return Calibrated::Refused(Refusal::OneSided);
    };
    if new_auc <= CHANCE_AUC {
        return Calibrated::Refused(Refusal::NoBetterThanChance { auc: new_auc });
    }
    if let Some(kept_auc) = kept.and_then(auc_of)
        && new_auc < kept_auc - AUC_TOLERANCE
    {
        return Calibrated::Refused(Refusal::WorseThanKept {
            auc: new_auc,
            kept_auc,
        });
    }
    Calibrated::Kept(Fitted {
        model,
        samples: u32::try_from(samples.len()).unwrap_or(u32::MAX),
        auc: new_auc,
    })
}

/// Fits a model in `transaction` at `now_ms` when it is due, keeping it as the newest when it is
/// kept: what became of the fitting, or none when none was due. One is due once there are
/// [`MIN_LABELLED`] labelled injections, and after that when more have been labelled since the
/// last fitting and [`REFIT_AFTER_MS`] has passed since it, or at once while the newest model kept
/// is one this build cannot use.
///
/// # Errors
///
/// The store's errors.
pub fn calibrate(
    transaction: &mut WriteTx<'_>,
    now_ms: i64,
) -> Result<Option<Calibrated>, StoreError> {
    let queries = transaction.queries();
    let labelled = queries.labelled_injection_count()?;
    let is_enough = usize::try_from(labelled).is_ok_and(|count| count >= MIN_LABELLED);
    let in_use = model_in_use(queries)?;
    let is_unusable = matches!(in_use, ModelInUse::Unusable { .. });
    let is_due = is_unusable
        || queries.calibration_attempt()?.is_none_or(|last| {
            labelled != last.labelled && now_ms.saturating_sub(last.at_ms) >= REFIT_AFTER_MS
        });
    if !is_enough || !is_due {
        return Ok(None);
    }
    let window = queries.labelled_injections(CALIBRATION_WINDOW)?;
    let calibrated = calibrated(&window, in_use.model());
    if let Calibrated::Kept(fitted) = calibrated {
        let coefficients = fitted.model.coefficients();
        transaction.add_calibration(
            (&coefficients, fitted.samples, fitted.auc, now_ms),
            KEPT_MODELS,
        )?;
    }
    transaction.set_calibration_attempt(&CalibrationAttempt {
        labelled,
        at_ms: now_ms,
    })?;
    Ok(Some(calibrated))
}

/// The confidence model the store keeps, as this build can use it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModelInUse {
    /// None is kept yet: the gate keeps its cold-start rule.
    None,
    /// The newest model kept, which decides.
    Kept {
        /// The model.
        model: ConfidenceModel,
        /// Its version.
        version: u32,
        /// How many labelled injections it was fitted on.
        samples: u32,
        /// Its AUC on the labelled injections held out from its fitting.
        auc: f64,
    },
    /// The newest model kept cannot be used by this build: it was fitted on other inputs than this
    /// build reads, or its row cannot be read back. The gate keeps its cold-start rule until upkeep
    /// fits a model this build can use.
    Unusable {
        /// Its version, as stored.
        version: i64,
    },
}

impl ModelInUse {
    /// The model that decides, when one does.
    #[must_use]
    pub fn model(&self) -> Option<&ConfidenceModel> {
        match self {
            Self::Kept { model, .. } => Some(model),
            Self::None | Self::Unusable { .. } => None,
        }
    }
}

/// The confidence model `queries` finds kept, as this build can use it. A row that cannot be read
/// back is a model that cannot be used, not an error: what the gate does then is said, and the next
/// fitting replaces it.
///
/// # Errors
///
/// The store's errors, but those of a calibration row that cannot be read back.
pub fn model_in_use(queries: cortex_store::Queries<'_>) -> Result<ModelInUse, StoreError> {
    match queries.calibration() {
        Ok(None) => Ok(ModelInUse::None),
        Ok(Some(record)) => Ok(
            match ConfidenceModel::from_coefficients(&record.coefficients) {
                Some(model) => ModelInUse::Kept {
                    model,
                    version: record.version,
                    samples: record.samples,
                    auc: record.auc,
                },
                None => ModelInUse::Unusable {
                    version: i64::from(record.version),
                },
            },
        ),
        Err(StoreError::Corrupt { column, row }) if column.starts_with("calibration.") => {
            Ok(ModelInUse::Unusable { version: row })
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
