//! Fitting the confidence model in the store as upkeep does: not before there are enough labelled
//! injections, then at most once a day and only when more were labelled, the model kept read back
//! as the one in use.

mod common;

use common::{Store, TestResult, start};
use cortex_core::{ContentHash, GateFeatures, HookKind, Identifier, StartSource};
use cortex_learn::{Calibrated, Commands, REFIT_AFTER_MS, Recorder, calibrate, model_in_use};
use cortex_store::{CalibrationAttempt, InjectionRecord};

/// Records `count` injections of prompts in session `s0`, numbered from `first`, each labelled used
/// exactly when its top lexical score, (37 n mod 100) / 100 for the n-th, is above 0.5.
fn labelled(store: &mut Store, first: u32, count: u32) -> TestResult {
    let mut transaction = store.writer.transaction()?;
    for at in first..first + count {
        let top_lexical = f64::from(at * 37 % 100) / 100.0;
        let id = transaction.record_injection(&InjectionRecord {
            task_id: None,
            session_id: Identifier::new("s0")?,
            hook: HookKind::Prompt,
            at_ms: 200 + i64::from(at),
            files: Vec::new(),
            tokens: 10,
            confidence: None,
            is_gated_out: false,
            is_holdout: false,
            text_hash: Some(ContentHash::of(b"a work order")),
            features: Some(GateFeatures {
                top_lexical,
                top_mass: 0.5,
                agreeing_kinds: 2,
                supporting_kinds: 2,
                similar_task: 0.0,
                recent_precision: None,
                is_continuing: false,
            }),
        })?;
        transaction.label_injection(id, 0, top_lexical > 0.5)?;
    }
    transaction.commit()?;
    Ok(())
}

/// Calibrates `store` at `now_ms` in a transaction of its own.
fn calibrated(store: &mut Store, now_ms: i64) -> TestResult<Option<Calibrated>> {
    let mut transaction = store.writer.transaction()?;
    let calibrated = calibrate(&mut transaction, now_ms)?;
    transaction.commit()?;
    Ok(calibrated)
}

/// With 199 labelled injections nothing is fitted and nothing noted. With 250, the first fitting
/// keeps a model (the newest 50 are ranked perfectly, an AUC of 1, by a score the labels follow),
/// version 1, read back as the model in use, and notes 250 at that time. Asked again with nothing
/// new, nothing is fitted. One more labelled a moment less than a day later: still nothing; a day
/// after the last fitting, a second model is fitted and kept.
#[test]
fn a_model_is_fitted_once_enough_are_labelled_and_then_daily() -> TestResult {
    let mut store = Store::new()?;
    let mut recorder = Recorder::new(false, 0.0, Commands::default());
    store.record(&mut recorder, &[start("s0", 100, StartSource::Startup)?])?;
    labelled(&mut store, 0, 199)?;
    assert_eq!(calibrated(&mut store, 1_000)?, None);
    assert_eq!(store.writer.queries().calibration_attempt()?, None);
    assert_eq!(model_in_use(store.writer.queries())?, None);
    labelled(&mut store, 199, 51)?;
    let Some(Calibrated::Kept(first)) = calibrated(&mut store, 2_000)? else {
        return Err("a first model kept".into());
    };
    assert!((first.auc - 1.0).abs() < f64::EPSILON, "{first:?}");
    assert_eq!(first.samples, 200);
    assert_eq!(model_in_use(store.writer.queries())?, Some(first.model));
    let kept = store
        .writer
        .queries()
        .calibration()?
        .ok_or("a kept model")?;
    assert_eq!((kept.version, kept.trained_at_ms), (1, 2_000));
    assert_eq!(
        store.writer.queries().calibration_attempt()?,
        Some(CalibrationAttempt {
            labelled: 250,
            at_ms: 2_000
        })
    );
    assert_eq!(calibrated(&mut store, 3_000)?, None);
    labelled(&mut store, 250, 1)?;
    assert_eq!(calibrated(&mut store, 2_000 + REFIT_AFTER_MS - 1)?, None);
    assert!(matches!(
        calibrated(&mut store, 2_000 + REFIT_AFTER_MS)?,
        Some(Calibrated::Kept(_))
    ));
    let newest = store
        .writer
        .queries()
        .calibration()?
        .ok_or("a second model")?;
    assert_eq!(newest.version, 2);
    Ok(())
}
