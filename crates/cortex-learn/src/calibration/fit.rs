//! Fitting the confidence model: logistic regression with an L2 penalty on the weights (not on the
//! intercept), by Newton's method. The loss is convex and the penalty makes it strictly so, so each
//! step solves a small symmetric positive definite system, halved until the loss does not rise, and
//! the fit stops once a step moves no coefficient by more than [`CONVERGED_STEP`]. Every sum runs
//! in the samples' order, so the same samples give the same model on every machine.

use super::model::{ConfidenceModel, MODEL_INPUTS, logistic};

/// How strongly the weights are pulled toward 0, against the summed log loss of the samples: with
/// hundreds of samples, a penalty of 1 barely moves weights the data supports, keeps those it does
/// not near 0, and keeps the fit finite when one feature alone separates the labels.
pub const L2_PENALTY: f64 = 1.0;

/// A step that moves no coefficient by more than this ends the fit: well below anything that
/// changes a probability the gate compares.
const CONVERGED_STEP: f64 = 1e-10;

/// The most Newton steps taken. The fit converges in under twenty on the samples Cortex keeps; the
/// bound only stops a fit that cannot.
const MAX_NEWTON_STEPS: usize = 100;

/// The most times a step is halved looking for one that does not raise the loss.
const MAX_HALVINGS: usize = 40;

/// How many coefficients the model has: the intercept and one weight per input.
const COEFFICIENTS: usize = MODEL_INPUTS + 1;

/// A square matrix of one row and one column per coefficient.
type Matrix = [[f64; COEFFICIENTS]; COEFFICIENTS];

/// One sample: the model's inputs and whether the injection was used.
pub(super) type Sample = ([f64; MODEL_INPUTS], bool);

/// The model fitted to `samples` with the weights' penalty `penalty`; none when the fit gives a
/// number that is not finite or a system that cannot be solved.
pub(super) fn fit(samples: &[Sample], penalty: f64) -> Option<ConfidenceModel> {
    let mut coefficients = [0.0; COEFFICIENTS];
    let mut loss = penalized_loss(samples, &coefficients, penalty);
    for _ in 0..MAX_NEWTON_STEPS {
        let (gradient, curvature) = derivatives(samples, &coefficients, penalty);
        let step = solved(&curvature, gradient)?;
        let Some((next, next_loss, moved)) =
            descent(samples, (&coefficients, loss), &step, penalty)
        else {
            // No fraction of the step lowers the loss: the fit is at its least, to the precision of
            // the numbers.
            break;
        };
        coefficients = next;
        loss = next_loss;
        if moved < CONVERGED_STEP {
            break;
        }
    }
    ConfidenceModel::from_coefficients(&coefficients)
}

/// The first of the step `step` taken from `at` (coefficients and their loss), halved as needed,
/// whose loss is no higher: the coefficients, their loss and the most any one moved.
fn descent(
    samples: &[Sample],
    (at, loss): (&[f64; COEFFICIENTS], f64),
    step: &[f64; COEFFICIENTS],
    penalty: f64,
) -> Option<([f64; COEFFICIENTS], f64, f64)> {
    let mut scale = 1.0;
    for _ in 0..MAX_HALVINGS {
        let mut next = *at;
        for (value, change) in next.iter_mut().zip(step) {
            *value -= scale * change;
        }
        let next_loss = penalized_loss(samples, &next, penalty);
        if next_loss <= loss {
            let moved = step
                .iter()
                .map(|change| (scale * change).abs())
                .fold(0.0, f64::max);
            return Some((next, next_loss, moved));
        }
        scale /= 2.0;
    }
    None
}

/// The coefficients split into the model they make, whatever their values.
fn model_of(coefficients: &[f64; COEFFICIENTS]) -> ConfidenceModel {
    let (&intercept, rest) = coefficients.split_first().unwrap_or((&0.0, &[]));
    let mut weights = [0.0; MODEL_INPUTS];
    for (weight, value) in weights.iter_mut().zip(rest) {
        *weight = *value;
    }
    ConfidenceModel { intercept, weights }
}

/// The summed log loss of `samples` under `coefficients`, plus half of `penalty` times the sum of
/// the squared weights.
fn penalized_loss(samples: &[Sample], coefficients: &[f64; COEFFICIENTS], penalty: f64) -> f64 {
    let model = model_of(coefficients);
    let log_loss: f64 = samples
        .iter()
        .map(|(inputs, is_used)| {
            let log_odds = model.log_odds(inputs);
            // The log loss -[y ln p + (1 - y) ln (1 - p)] is softplus(z) - y z, and softplus is
            // written so that a large |z| neither overflows nor rounds to 0.
            let softplus = log_odds.max(0.0) + (-log_odds.abs()).exp().ln_1p();
            softplus - if *is_used { log_odds } else { 0.0 }
        })
        .sum();
    let squared: f64 = model.weights.iter().map(|weight| weight * weight).sum();
    log_loss + penalty / 2.0 * squared
}

/// The gradient and the matrix of second derivatives of the penalized loss at `coefficients`.
fn derivatives(
    samples: &[Sample],
    coefficients: &[f64; COEFFICIENTS],
    penalty: f64,
) -> ([f64; COEFFICIENTS], Matrix) {
    let model = model_of(coefficients);
    let mut gradient = [0.0; COEFFICIENTS];
    let mut curvature: Matrix = [[0.0; COEFFICIENTS]; COEFFICIENTS];
    for (inputs, is_used) in samples {
        let probability = logistic(model.log_odds(inputs));
        let error = probability - if *is_used { 1.0 } else { 0.0 };
        let weight = probability * (1.0 - probability);
        let row = with_intercept(inputs);
        for ((slope, line), left) in gradient.iter_mut().zip(curvature.iter_mut()).zip(&row) {
            *slope += error * left;
            for (cell, right) in line.iter_mut().zip(&row) {
                *cell += weight * left * right;
            }
        }
    }
    // The intercept, first, is not penalized.
    for (at, (slope, line)) in gradient
        .iter_mut()
        .zip(curvature.iter_mut())
        .enumerate()
        .skip(1)
    {
        *slope += penalty * coefficients.get(at).copied().unwrap_or(0.0);
        if let Some(diagonal) = line.get_mut(at) {
            *diagonal += penalty;
        }
    }
    (gradient, curvature)
}

/// `inputs` with a 1 before them, for the intercept.
fn with_intercept(inputs: &[f64; MODEL_INPUTS]) -> [f64; COEFFICIENTS] {
    let mut row = [1.0; COEFFICIENTS];
    for (cell, input) in row.iter_mut().skip(1).zip(inputs) {
        *cell = *input;
    }
    row
}

/// The solution `x` of `matrix x = vector` for a symmetric positive definite `matrix`, by its
/// Cholesky factor; none when the matrix is not positive definite to the precision of the numbers.
// Every index below runs over 0..COEFFICIENTS on arrays of exactly that size.
#[allow(clippy::indexing_slicing)]
fn solved(matrix: &Matrix, vector: [f64; COEFFICIENTS]) -> Option<[f64; COEFFICIENTS]> {
    let mut lower: Matrix = [[0.0; COEFFICIENTS]; COEFFICIENTS];
    for row in 0..COEFFICIENTS {
        for column in 0..=row {
            let known: f64 = (0..column)
                .map(|inner| lower[row][inner] * lower[column][inner])
                .sum();
            let rest = matrix[row][column] - known;
            if row == column {
                if rest <= 0.0 || !rest.is_finite() {
                    return None;
                }
                lower[row][column] = rest.sqrt();
            } else {
                lower[row][column] = rest / lower[column][column];
            }
        }
    }
    let mut forward = [0.0; COEFFICIENTS];
    for row in 0..COEFFICIENTS {
        let known: f64 = (0..row)
            .map(|inner| lower[row][inner] * forward[inner])
            .sum();
        forward[row] = (vector[row] - known) / lower[row][row];
    }
    let mut solution = [0.0; COEFFICIENTS];
    for row in (0..COEFFICIENTS).rev() {
        let known: f64 = (row + 1..COEFFICIENTS)
            .map(|inner| lower[inner][row] * solution[inner])
            .sum();
        solution[row] = (forward[row] - known) / lower[row][row];
    }
    solution
        .iter()
        .all(|value| value.is_finite())
        .then_some(solution)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sample whose first input is `first` and every other input 0.
    fn sample(first: f64, is_used: bool) -> Sample {
        let mut inputs = [0.0; MODEL_INPUTS];
        inputs[0] = first;
        (inputs, is_used)
    }

    /// One binary input has a closed form without a penalty: the fitted probability of each group
    /// is its share used. Where the input is 0, 3 of 4 were used, so the intercept is ln(0.75 /
    /// 0.25) = ln 3, about 1.098612; where it is 1, 1 of 4, so intercept plus weight is ln(1/3) and
    /// the weight is -2 ln 3, about -2.197225. A penalty of 1e-9 is too small to move either by
    /// 1e-6, and the inputs that are always 0 keep weights of 0.
    #[test]
    fn one_binary_input_fits_its_closed_form() -> Result<(), &'static str> {
        let samples = [
            sample(0.0, true),
            sample(0.0, true),
            sample(0.0, true),
            sample(0.0, false),
            sample(1.0, true),
            sample(1.0, false),
            sample(1.0, false),
            sample(1.0, false),
        ];
        let model = fit(&samples, 1e-9).ok_or("a model")?;
        let ln_3 = 3.0_f64.ln();
        assert!((model.intercept - ln_3).abs() < 1e-6, "{model:?}");
        let [first, rest @ ..] = model.weights;
        assert!((first + 2.0 * ln_3).abs() < 1e-6, "{model:?}");
        assert!(rest.iter().all(|weight| weight.abs() < 1e-9), "{model:?}");
        let penalized = fit(&samples, L2_PENALTY).ok_or("a penalized model")?;
        let [shrunk, ..] = penalized.weights;
        assert!(shrunk < 0.0 && shrunk > -2.0 * ln_3, "{penalized:?}");
        Ok(())
    }

    /// Labels a single input separates perfectly still give a finite model under the penalty,
    /// whose probabilities fall on either side of one half; the same samples give the same model
    /// twice, to the bit.
    #[test]
    fn separated_labels_fit_finitely_and_deterministically() -> Result<(), &'static str> {
        let samples: Vec<Sample> = (0..40)
            .map(|at| {
                let first = f64::from(at) / 40.0;
                sample(first, first > 0.5)
            })
            .collect();
        let model = fit(&samples, L2_PENALTY).ok_or("a model")?;
        assert!(model.is_finite());
        assert!(logistic(model.log_odds(&sample(0.9, true).0)) > 0.5);
        assert!(logistic(model.log_odds(&sample(0.1, false).0)) < 0.5);
        assert_eq!(fit(&samples, L2_PENALTY), Some(model));
        Ok(())
    }

    /// The solver's answer to a system worked by hand: [[4, 2], [2, 3]] x = [2, 5] in the first
    /// two coefficients, the identity elsewhere with 1s on the right, gives x = (-0.5, 2) there
    /// (4 x -0.5 + 2 x 2 = 2, 2 x -0.5 + 3 x 2 = 5) and 1 elsewhere. A matrix with a zero on its
    /// diagonal is not positive definite.
    #[test]
    fn the_solver_solves_a_system_worked_by_hand() -> Result<(), &'static str> {
        let mut matrix: Matrix = [[0.0; COEFFICIENTS]; COEFFICIENTS];
        for (at, line) in matrix.iter_mut().enumerate() {
            line[at] = 1.0;
        }
        matrix[0][0] = 4.0;
        matrix[0][1] = 2.0;
        matrix[1][0] = 2.0;
        matrix[1][1] = 3.0;
        let mut vector = [1.0; COEFFICIENTS];
        vector[0] = 2.0;
        vector[1] = 5.0;
        let solution = solved(&matrix, vector).ok_or("a solution")?;
        let mut expected = [1.0; COEFFICIENTS];
        expected[0] = -0.5;
        expected[1] = 2.0;
        for (got, want) in solution.iter().zip(expected) {
            assert!((got - want).abs() < 1e-12, "{solution:?}");
        }
        matrix[2][2] = 0.0;
        assert_eq!(solved(&matrix, vector), None);
        Ok(())
    }
}
