//! Fitting and keeping on hand-built windows whose answers are worked out from how they are built.

use cortex_core::GateFeatures;

use super::*;

/// A labelled injection `id` whose top lexical score is `top_lexical`, whose top files hold
/// `top_mass` of the activation, and whose task used it when `label`; its other features are the
/// same for every injection here.
fn labelled(id: usize, (top_lexical, top_mass): (f64, f64), label: bool) -> LabelledInjection {
    LabelledInjection {
        id: i64::try_from(id).unwrap_or(i64::MAX),
        features: GateFeatures {
            top_lexical,
            top_mass,
            agreeing_kinds: 2,
            supporting_kinds: 2,
            similar_task: 0.0,
            recent_precision: Some(0.75),
            is_continuing: false,
        },
        label,
    }
}

/// `count` injections used exactly when their top lexical score is above 0.5, the scores spread
/// over 0 to 0.99 (the score of injection `i` is (37 i mod 100) / 100), their top mass 0.5.
fn ruled(count: usize) -> Vec<LabelledInjection> {
    (0..count)
        .map(|at| {
            let top_lexical = f64::from(u32::try_from(at * 37 % 100).unwrap_or(0)) / 100.0;
            labelled(at, (top_lexical, 0.5), top_lexical > 0.5)
        })
        .collect()
}

/// The 40 injections held out of a 200-long window, built so that a model reading the top lexical
/// score ranks them at a known AUC and one reading the top mass ranks them perfectly: 20 used at
/// lexical 0.6 and mass 0.9, then `low` unused at lexical 0.2 and the rest of 20 unused at lexical
/// 0.8, all unused at mass 0.1.
fn held_out(low: usize) -> Vec<LabelledInjection> {
    let used = (0..20).map(|at| labelled(160 + at, (0.6, 0.9), true));
    let unused = (0..20).map(|at| {
        let top_lexical = if at < low { 0.2 } else { 0.8 };
        labelled(180 + at, (top_lexical, 0.1), false)
    });
    used.chain(unused).collect()
}

/// A model that reads only the top mass: log-odds 10 x mass - 5.
fn mass_model() -> ConfidenceModel {
    ConfidenceModel {
        intercept: -5.0,
        weights: [0.0, 10.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    }
}

/// A window of 250 injections used exactly when their top lexical score is above 0.5: the oldest
/// 200 fit the model and the newest 50 judge it. The model's probability rises with the score
/// alone, so it ranks the 50 perfectly, an AUC of 1, and is kept with nothing kept before. It puts
/// a score of 0.9 at 0.7 or more and one of 0.1 at 0.3 or less, either side of the gate's bar. A
/// second fitting on the same window, with that model kept, scores the same AUC and is kept again.
#[test]
fn a_known_separating_rule_is_learned_and_kept() -> Result<(), String> {
    let window = ruled(250);
    let Calibrated::Kept(fitted) = calibrated(&window, None) else {
        return Err(format!("{:?}", calibrated(&window, None)));
    };
    assert_eq!(fitted.samples, 200);
    assert!((fitted.auc - 1.0).abs() < f64::EPSILON, "{fitted:?}");
    let high = fitted
        .model
        .probability(&labelled(0, (0.9, 0.5), true).features);
    let low = fitted
        .model
        .probability(&labelled(0, (0.1, 0.5), false).features);
    assert!(high >= 0.7, "{high}");
    assert!(low <= 0.3, "{low}");
    assert!(matches!(
        calibrated(&window, Some(&fitted.model)),
        Calibrated::Kept(again) if (again.auc - 1.0).abs() < f64::EPSILON
    ));
    Ok(())
}

/// The 160 oldest of a 200-long window follow the top lexical score with a constant top mass, so
/// the model fitted on them reads the score alone. On the 40 held out, the 20 used at lexical 0.6
/// rank above the 15 unused at 0.2 and below the 5 at 0.8: 20 x 15 of 20 x 20 pairs right, an AUC
/// of 0.75. A kept model reading the top mass ranks those 40 perfectly, an AUC of 1, so the new
/// one is more than 0.01 worse and refused. With no model kept, 0.75 beats chance and is kept.
#[test]
fn a_model_worse_than_the_kept_one_is_refused() {
    let mut window = ruled(160);
    window.extend(held_out(15));
    assert_eq!(
        calibrated(&window, Some(&mass_model())),
        Calibrated::Refused(Refusal::WorseThanKept {
            auc: 0.75,
            kept_auc: 1.0,
        })
    );
    assert!(matches!(
        calibrated(&window, None),
        Calibrated::Kept(Fitted { auc, samples: 160, .. }) if (auc - 0.75).abs() < f64::EPSILON
    ));
}

/// Held out as above with 10 unused at lexical 0.2 and 10 at 0.8, the used at 0.6 rank right in
/// 20 x 10 of 400 pairs: an AUC of 0.5, no better than chance, refused even with nothing kept.
#[test]
fn a_model_no_better_than_chance_is_refused() {
    let mut window = ruled(160);
    window.extend(held_out(10));
    assert_eq!(
        calibrated(&window, None),
        Calibrated::Refused(Refusal::NoBetterThanChance { auc: 0.5 })
    );
}

/// 199 labelled injections are too few. 200 all used, or 200 whose newest 40 are all unused, give
/// one side only where a model is fitted or judged.
#[test]
fn too_few_or_one_sided_windows_fit_nothing() {
    assert_eq!(
        calibrated(&ruled(199), None),
        Calibrated::Refused(Refusal::TooFew { labelled: 199 })
    );
    let all_used: Vec<LabelledInjection> =
        (0..200).map(|at| labelled(at, (0.9, 0.5), true)).collect();
    assert_eq!(
        calibrated(&all_used, None),
        Calibrated::Refused(Refusal::OneSided)
    );
    let mut newest_unused = ruled(160);
    newest_unused.extend((160..200).map(|at| labelled(at, (0.2, 0.5), false)));
    assert_eq!(
        calibrated(&newest_unused, None),
        Calibrated::Refused(Refusal::OneSided)
    );
}
