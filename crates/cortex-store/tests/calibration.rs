//! The confidence models kept and what they are fitted on: the labelled injections of prompts that
//! carry the gate's features, and the last time a model was fitted.

mod common;

use common::{Scratch, TestResult};
use cortex_core::{Agent, ContentHash, GateFeatures, HookKind, Identifier, Ulid};
use cortex_store::{
    CalibrationAttempt, CalibrationRecord, InjectionRecord, SessionRecord, StoreError, TaskRecord,
    Writer,
};

/// Records session `s1` and, in it, `count` tasks, returned in order.
fn tasks(writer: &mut Writer, count: u8) -> TestResult<Vec<Ulid>> {
    let tasks: Vec<Ulid> = (1..=count)
        .map(|at| Ulid::from_seed(u64::from(at), b"calibration"))
        .collect();
    let mut transaction = writer.transaction()?;
    transaction.record_session(&SessionRecord {
        session_id: Identifier::new("s1")?,
        agent: Agent::Claude,
        agent_version: None,
        model: None,
        parent_session_id: None,
        cwd: None,
        transcript_path: None,
        started_at_ms: 500,
    })?;
    for (at, task) in tasks.iter().enumerate() {
        transaction.start_task(&TaskRecord {
            task_id: *task,
            session_id: Identifier::new("s1")?,
            started_at_ms: 1_000 + i64::try_from(at)?,
            prompt_hash: None,
            prompt_excerpt: None,
            holdout: Some(false),
        })?;
    }
    transaction.commit()?;
    Ok(tasks)
}

/// The gate's numbers of one retrieval, told apart by its top lexical score.
fn features(top_lexical: f64) -> GateFeatures {
    GateFeatures {
        top_lexical,
        top_mass: 0.5,
        agreeing_kinds: 2,
        supporting_kinds: 2,
        similar_task: 0.0,
        recent_precision: Some(0.75),
        is_continuing: false,
    }
}

/// Of seven injections, recorded in this order: a delivered prompt's at lexical 0.1 labelled used;
/// one the gate kept back at 0.2 labelled unused; a held-out task's at 0.3 labelled used; a
/// prompt's with no features labelled used; a prompt's at 0.4 never labelled; a stop's reason at
/// 0.5 labelled used; a delivered prompt's at 0.6 labelled unused. The labelled prompts with
/// features are the first, second, third and seventh: four, read oldest first (0.1, 0.2, 0.3,
/// 0.6), and the latest two of them are 0.3 and 0.6, still oldest first.
#[test]
fn the_labelled_injections_are_the_latest_prompts_with_features() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let tasks = tasks(&mut writer, 7)?;
    assert!(writer.queries().labelled_injections(10)?.is_empty());
    assert_eq!(writer.queries().labelled_injection_count()?, 0);
    let session = Identifier::new("s1")?;
    let given = |task: Ulid, features: Option<GateFeatures>| InjectionRecord {
        task_id: Some(task),
        session_id: session.clone(),
        hook: HookKind::Prompt,
        at_ms: 2_000,
        files: Vec::new(),
        tokens: 20,
        confidence: None,
        is_gated_out: false,
        is_holdout: false,
        text_hash: Some(ContentHash::of(b"the work order")),
        features,
    };
    let rows = [
        (given(tasks[0], Some(features(0.1))), Some(true)),
        (
            InjectionRecord {
                is_gated_out: true,
                text_hash: None,
                ..given(tasks[1], Some(features(0.2)))
            },
            Some(false),
        ),
        (
            InjectionRecord {
                is_holdout: true,
                ..given(tasks[2], Some(features(0.3)))
            },
            Some(true),
        ),
        (given(tasks[3], None), Some(true)),
        (given(tasks[4], Some(features(0.4))), None),
        (
            InjectionRecord {
                hook: HookKind::Stop,
                ..given(tasks[5], Some(features(0.5)))
            },
            Some(true),
        ),
        (given(tasks[6], Some(features(0.6))), Some(false)),
    ];
    let mut transaction = writer.transaction()?;
    let mut ids = Vec::new();
    for (record, label) in &rows {
        let id = transaction.record_injection(record)?;
        ids.push(id.get());
        if let Some(label) = label {
            transaction.label_injection(id, 0, *label)?;
        }
    }
    transaction.commit()?;
    let queries = writer.queries();
    let read = |limit| -> TestResult<Vec<(i64, f64, bool)>> {
        Ok(queries
            .labelled_injections(limit)?
            .into_iter()
            .map(|labelled| (labelled.id, labelled.features.top_lexical, labelled.label))
            .collect())
    };
    assert_eq!(
        read(10)?,
        [
            (ids[0], 0.1, true),
            (ids[1], 0.2, false),
            (ids[2], 0.3, true),
            (ids[6], 0.6, false),
        ]
    );
    assert_eq!(read(2)?, [(ids[2], 0.3, true), (ids[6], 0.6, false)]);
    assert_eq!(queries.labelled_injection_count()?, 4);
    let first = queries
        .labelled_injections(1)?
        .into_iter()
        .next()
        .ok_or("one")?;
    assert_eq!(first.features, features(0.6));
    Ok(())
}

/// Coefficients read back exactly as they were kept, to the last bit: 0.1 + 0.2 (written
/// 0.30000000000000004) and two weights of about -1.06e-22, written ...828e-22 and ...289e-22,
/// that a parser one unit in the last place off reads as ...829e-22 and ...288e-22.
#[test]
fn coefficients_read_back_to_the_bit() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    let coefficients = [
        0.1 + 0.2,
        -1.058_780_894_363_382_8e-22,
        -1.058_785_723_178_228_9e-22,
    ];
    let mut transaction = writer.transaction()?;
    transaction.add_calibration((&coefficients, 200, 1.0, 1_000), 1)?;
    transaction.commit()?;
    let read = writer.queries().calibration()?.ok_or("a model")?;
    let bits = |values: &[f64]| {
        values
            .iter()
            .map(|value| value.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&read.coefficients), bits(&coefficients));
    Ok(())
}

/// Models are versioned from 1 and the newest is read back as kept; keeping the newest two, the
/// third deletes the first. The last fitting reads back as kept. Coefficients that are not a list
/// of numbers are an error naming the column and the version, not a model.
#[test]
fn models_are_kept_newest_first_and_the_old_ones_go() -> TestResult {
    let scratch = Scratch::new()?;
    let mut writer = scratch.writer(1)?;
    assert_eq!(writer.queries().calibration()?, None);
    assert_eq!(writer.queries().calibration_attempt()?, None);
    let models: [(&[f64], u32, f64, i64); 3] = [
        (&[0.5, -1.0, 2.0], 160, 0.75, 1_000),
        (&[1.0], 200, 0.8, 2_000),
        (&[-0.25, 0.125], 240, 0.625, 3_000),
    ];
    for (at, model) in models.into_iter().enumerate() {
        let mut transaction = writer.transaction()?;
        let version = transaction.add_calibration(model, 2)?;
        transaction.commit()?;
        assert_eq!(usize::try_from(version)?, at + 1);
    }
    assert_eq!(
        writer.queries().calibration()?,
        Some(CalibrationRecord {
            version: 3,
            coefficients: vec![-0.25, 0.125],
            samples: 240,
            auc: 0.625,
            trained_at_ms: 3_000,
        })
    );
    let attempt = CalibrationAttempt {
        labelled: 250,
        at_ms: 3_000,
    };
    let mut transaction = writer.transaction()?;
    transaction.set_calibration_attempt(&attempt)?;
    transaction.commit()?;
    assert_eq!(writer.queries().calibration_attempt()?, Some(attempt));
    drop(writer);
    let connection = rusqlite::Connection::open(scratch.folder.join("cortex.db"))?;
    let versions: Vec<i64> = connection
        .prepare("SELECT version FROM calibration ORDER BY version")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    assert_eq!(versions, [2, 3]);
    connection
        .execute_batch("UPDATE calibration SET coefficients = '[\"x\"]' WHERE version = 3")?;
    let read = scratch.reader()?.queries().calibration();
    assert!(
        matches!(
            read,
            Err(StoreError::Corrupt {
                column: "calibration.coefficients",
                row: 3
            })
        ),
        "{read:?}"
    );
    Ok(())
}
