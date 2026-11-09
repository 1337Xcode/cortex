//! The confidence model: a logistic regression over the gate's features, which turns what one
//! retrieval found into the probability that its task uses one of the files it would list.

use cortex_core::GateFeatures;

/// How many numbers the model reads from the gate's features.
pub const MODEL_INPUTS: usize = 8;

/// The gate's features as the model reads them, in this order: the top lexical score, the top
/// files' share of the activation, the most kinds of evidence agreeing on one top file, the kinds
/// supporting any, how alike the prompt is to earlier work, the recent precision (0 when there is
/// none) and whether there is one (1 or 0), and whether the session is continuing its own work
/// (1 or 0). The counts of kinds are read as they are, from 0 to the number of kinds retrieval
/// counts, so that a kind added later needs no rescaling of the others.
#[must_use]
pub fn model_inputs(features: &GateFeatures) -> [f64; MODEL_INPUTS] {
    let flag = |is: bool| if is { 1.0 } else { 0.0 };
    [
        features.top_lexical,
        features.top_mass,
        f64::from(features.agreeing_kinds),
        f64::from(features.supporting_kinds),
        features.similar_task,
        features.recent_precision.unwrap_or(0.0),
        flag(features.recent_precision.is_some()),
        flag(features.is_continuing),
    ]
}

/// A fitted model: an intercept and one weight for each of [`model_inputs`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfidenceModel {
    /// The log-odds when every input is 0.
    pub intercept: f64,
    /// What each input adds to the log-odds per unit.
    pub weights: [f64; MODEL_INPUTS],
}

impl ConfidenceModel {
    /// The model whose coefficients, as the store keeps them, are `coefficients`: the intercept
    /// first, then the weights. None unless there are exactly one more than [`MODEL_INPUTS`] and
    /// each is finite, as when a store kept one by a build that read other inputs.
    #[must_use]
    pub fn from_coefficients(coefficients: &[f64]) -> Option<Self> {
        let (&intercept, rest) = coefficients.split_first()?;
        let weights: [f64; MODEL_INPUTS] = rest.try_into().ok()?;
        let model = Self { intercept, weights };
        model.is_finite().then_some(model)
    }

    /// The coefficients as the store keeps them: the intercept first, then the weights.
    #[must_use]
    pub fn coefficients(&self) -> Vec<f64> {
        std::iter::once(self.intercept)
            .chain(self.weights)
            .collect()
    }

    /// The probability, from 0 to 1, that a task uses one of the files a retrieval with
    /// `features` would list.
    #[must_use]
    pub fn probability(&self, features: &GateFeatures) -> f64 {
        logistic(self.log_odds(&model_inputs(features)))
    }

    /// The log-odds the model gives `inputs`.
    pub(super) fn log_odds(&self, inputs: &[f64; MODEL_INPUTS]) -> f64 {
        self.weights
            .iter()
            .zip(inputs)
            .fold(self.intercept, |sum, (weight, input)| sum + weight * input)
    }

    /// Whether every coefficient is a finite number.
    pub(super) fn is_finite(&self) -> bool {
        self.intercept.is_finite() && self.weights.iter().all(|weight| weight.is_finite())
    }
}

/// The logistic function of `log_odds`, from 0 to 1, computed so that a large magnitude neither
/// overflows nor loses the side it falls on.
pub(super) fn logistic(log_odds: f64) -> f64 {
    if log_odds >= 0.0 {
        1.0 / (1.0 + (-log_odds).exp())
    } else {
        let odds = log_odds.exp();
        odds / (1.0 + odds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Features with every number set apart, to see each lands in its place.
    fn features() -> GateFeatures {
        GateFeatures {
            top_lexical: 0.5,
            top_mass: 0.25,
            agreeing_kinds: 2,
            supporting_kinds: 3,
            similar_task: 0.125,
            recent_precision: Some(0.75),
            is_continuing: true,
        }
    }

    /// Each feature lands in its place; a missing recent precision reads 0 with its flag 0, and a
    /// session not continuing reads 0.
    #[test]
    fn the_inputs_are_the_features_in_order() {
        assert_eq!(
            model_inputs(&features()),
            [0.5, 0.25, 2.0, 3.0, 0.125, 0.75, 1.0, 1.0]
        );
        let without = GateFeatures {
            recent_precision: None,
            is_continuing: false,
            ..features()
        };
        assert_eq!(
            model_inputs(&without),
            [0.5, 0.25, 2.0, 3.0, 0.125, 0.0, 0.0, 0.0]
        );
    }

    /// With an intercept of -1 and weights of 2 on the top lexical score and 1 on continuing, the
    /// features above give log-odds -1 + 2 x 0.5 + 1 = 1, a probability of 1 / (1 + e^-1), about
    /// 0.7311; the coefficients go to the store and back unchanged. Nine coefficients make a model;
    /// eight, ten, or one that is not finite do not.
    #[test]
    fn a_model_gives_the_logistic_of_its_log_odds() {
        let model = ConfidenceModel {
            intercept: -1.0,
            weights: [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        };
        let probability = model.probability(&features());
        assert!(
            (probability - 0.731_058_578_6).abs() < 1e-9,
            "{probability}"
        );
        let coefficients = model.coefficients();
        assert_eq!(coefficients, [-1.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            ConfidenceModel::from_coefficients(&coefficients),
            Some(model)
        );
        assert_eq!(ConfidenceModel::from_coefficients(&coefficients[..8]), None);
        let mut ten = coefficients.clone();
        ten.push(0.0);
        assert_eq!(ConfidenceModel::from_coefficients(&ten), None);
        let mut infinite = coefficients;
        infinite[3] = f64::INFINITY;
        assert_eq!(ConfidenceModel::from_coefficients(&infinite), None);
        assert_eq!(ConfidenceModel::from_coefficients(&[]), None);
    }

    /// The logistic function is 0.5 at 0, symmetric, and stays within 0 to 1 without overflow at
    /// large magnitudes.
    #[test]
    fn the_logistic_function_is_bounded_and_symmetric() {
        assert!((logistic(0.0) - 0.5).abs() < f64::EPSILON);
        for log_odds in [0.5, 3.0, 40.0, 800.0] {
            let (up, down) = (logistic(log_odds), logistic(-log_odds));
            assert!((up + down - 1.0).abs() < 1e-12, "{log_odds}");
            assert!((0.0..=1.0).contains(&up) && (0.0..=1.0).contains(&down));
        }
        assert!(logistic(-800.0) >= 0.0 && logistic(800.0) <= 1.0);
    }
}
