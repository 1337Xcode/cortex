-- The confidence model the gate uses once there are enough labelled injections: one row for each
-- model kept, the newest the one in use. `coefficients` is a JSON array of numbers, the intercept
-- first, then one weight for each of the gate's features as the learner orders them. `samples` is
-- how many labelled injections it was fitted on, and `auc` how well it ranked the labelled
-- injections it was not fitted on. Numbers only: no prompt, path or code.

CREATE TABLE calibration (
    version INTEGER PRIMARY KEY,
    coefficients TEXT NOT NULL,
    samples INTEGER NOT NULL,
    auc REAL NOT NULL,
    trained_at INTEGER NOT NULL
) STRICT;
