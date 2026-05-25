//! Coverage command: cross-reference call graph with test coverage data.
//!
//! `cortex coverage --lcov coverage.lcov` reads LCOV/gcov output and ranks
//! untested functions by how many other functions call them. The most-called
//! untested function is your highest-risk coverage gap.
//!
//! Additionally, coverage data is written into each node's `attributes` JSON
//! under a `"coverage"` key containing `hit_count`, `line_coverage_pct`, and
//! `is_covered` fields.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::store::db::StoreManager;
use crate::store::types::CoverageData;

/// A single coverage gap entry for display.
struct CoverageGap {
    fqn: String,
    file: String,
    start_line: u32,
    caller_count: u32,
    status: String,
}

/// Per-line coverage information from LCOV.
struct LineCoverage {
    /// Lines with zero execution count (uncovered).
    uncovered: HashSet<u32>,
    /// Lines with non-zero execution count (covered), mapped to hit count.
    covered: HashMap<u32, u64>,
}

/// Parse an LCOV file and return a map of file -> line coverage data.
///
/// LCOV format:
///   SF:<source file path>
///   DA:<line number>,<execution count>
///   end_of_record
fn parse_lcov(path: &Path) -> Result<HashMap<String, LineCoverage>, anyhow::Error> {
    let content = fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!(
            "Cannot read LCOV file '{}': {}. \
             Make sure the file exists. Run your test suite with coverage enabled \
             to generate it (e.g., `cargo tarpaulin --out Lcov` for Rust, \
             `jest --coverage` for JS/TS, `pytest --cov` for Python).",
            path.display(),
            e
        )
    })?;

    let mut file_coverage: HashMap<String, LineCoverage> = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        if let Some(sf) = line.strip_prefix("SF:") {
            current_file = Some(sf.to_string());
        } else if let Some(da_content) = line.strip_prefix("DA:") {
            if let Some(ref file) = current_file {
                // DA:line_number,execution_count
                let parts: Vec<&str> = da_content.splitn(2, ',').collect();
                if parts.len() == 2
                    && let (Ok(line_num), Ok(count)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u64>())
                {
                    let entry = file_coverage
                        .entry(file.clone())
                        .or_insert_with(|| LineCoverage {
                            uncovered: HashSet::new(),
                            covered: HashMap::new(),
                        });
                    if count == 0 {
                        entry.uncovered.insert(line_num);
                    } else {
                        entry.covered.insert(line_num, count);
                    }
                }
            }
        } else if line == "end_of_record" {
            current_file = None;
        }
    }

    Ok(file_coverage)
}

/// Normalize a file path for comparison: strip leading ./ and use forward slashes.
fn normalize_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// Run the coverage analysis and print results.
pub fn run(store: &Arc<StoreManager>, lcov_path: &Path, limit: usize) -> Result<(), anyhow::Error> {
    // Parse LCOV data
    let file_coverage = parse_lcov(lcov_path)?;

    if file_coverage.is_empty() {
        println!("No coverage data found in LCOV file.");
        return Ok(());
    }

    let conn = store.read_conn();

    // Query all nodes from the database
    let mut stmt = conn.prepare(
        "SELECT fqn, kind, file, start_line, end_line FROM nodes ORDER BY file, start_line",
    )?;

    let nodes: Vec<(String, String, String, u32, u32)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // For each node, compute coverage data and collect gaps
    let mut gaps: Vec<CoverageGap> = Vec::new();
    let mut coverage_updates: Vec<(String, CoverageData)> = Vec::new();

    for (fqn, _kind, file, start_line, end_line) in &nodes {
        let norm_file = normalize_path(file);

        // Find matching LCOV file entry
        let lcov_entry = file_coverage.iter().find_map(|(lcov_file, line_cov)| {
            let norm_lcov = normalize_path(lcov_file);
            if norm_lcov == norm_file
                || norm_lcov.ends_with(&norm_file)
                || norm_file.ends_with(&norm_lcov)
            {
                Some(line_cov)
            } else {
                None
            }
        });

        if let Some(line_cov) = lcov_entry {
            // Compute coverage data for this node
            let total_lines = (end_line - start_line + 1) as usize;
            let mut covered_lines = 0usize;
            let mut total_hits: u64 = 0;

            for line_num in *start_line..=*end_line {
                if let Some(&hits) = line_cov.covered.get(&line_num) {
                    covered_lines += 1;
                    total_hits += hits;
                }
            }

            let line_coverage_pct = if total_lines > 0 {
                (covered_lines as f64 / total_lines as f64) * 100.0
            } else {
                0.0
            };

            let hit_count = total_hits.min(u32::MAX as u64) as u32;
            let is_covered = hit_count > 0;

            let coverage_data = CoverageData {
                hit_count,
                line_coverage_pct,
                is_covered,
            };

            coverage_updates.push((fqn.clone(), coverage_data));

            // Check if any line in the node's range is uncovered (for gap reporting)
            let has_uncovered = (*start_line..=*end_line).any(|l| line_cov.uncovered.contains(&l));

            if has_uncovered {
                let status = if covered_lines == 0 {
                    "UNCOVERED".to_string()
                } else {
                    format!("PARTIAL ({}/{})", covered_lines, total_lines)
                };

                gaps.push(CoverageGap {
                    fqn: fqn.clone(),
                    file: file.clone(),
                    start_line: *start_line,
                    caller_count: 0,
                    status,
                });
            }
        }
    }

    // Write coverage data into node attributes
    drop(stmt); // Release statement before dropping connection
    drop(conn); // Release read connection before writing
    write_coverage_to_store(store, &coverage_updates)?;

    if gaps.is_empty() {
        println!("All indexed functions have full test coverage. No gaps found.");
        return Ok(());
    }

    // Count callers for each gap node
    let conn = store.read_conn();
    let mut caller_counts: HashMap<String, u32> = HashMap::new();
    let mut count_stmt = conn.prepare(
        "SELECT target_fqn, COUNT(*) FROM edges WHERE kind = 'Calls' GROUP BY target_fqn",
    )?;

    let rows = count_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;

    for (fqn, count) in rows.flatten() {
        caller_counts.insert(fqn, count);
    }

    // Assign caller counts to gaps
    for gap in &mut gaps {
        gap.caller_count = caller_counts.get(&gap.fqn).copied().unwrap_or(0);
    }

    // Sort by caller count descending (highest risk first)
    gaps.sort_by_key(|g| std::cmp::Reverse(g.caller_count));

    // Limit results
    gaps.truncate(limit);

    // Print results
    println!();
    println!("COVERAGE GAPS (ranked by risk)");
    println!("{}", "━".repeat(90));
    println!();
    println!(
        "  {:<40} {:<25} {:>5} {:>8} Coverage",
        "Function", "File", "Line", "Callers"
    );
    println!(
        "  {:<40} {:<25} {:>5} {:>8} ────────",
        "────────", "────", "────", "───────"
    );

    for gap in &gaps {
        let short_fqn = if gap.fqn.len() > 38 {
            format!("..{}", &gap.fqn[gap.fqn.len() - 36..])
        } else {
            gap.fqn.clone()
        };

        let short_file = if gap.file.len() > 23 {
            format!("..{}", &gap.file[gap.file.len() - 21..])
        } else {
            gap.file.clone()
        };

        println!(
            "  {:<40} {:<25} {:>5} {:>8} {}",
            short_fqn, short_file, gap.start_line, gap.caller_count, gap.status
        );
    }

    println!();
    println!("{}", "━".repeat(90));
    println!(
        "  {} coverage gap{} found. Functions with more callers are higher risk.",
        gaps.len(),
        if gaps.len() == 1 { "" } else { "s" }
    );
    println!(
        "  Coverage data written to {} node attributes.",
        coverage_updates.len()
    );
    println!();

    Ok(())
}

/// Write coverage data into node attributes in the store.
///
/// For each node, reads the current `attributes` JSON, inserts/updates the
/// `"coverage"` key with the `CoverageData`, and writes it back.
fn write_coverage_to_store(
    store: &Arc<StoreManager>,
    updates: &[(String, CoverageData)],
) -> Result<(), anyhow::Error> {
    let conn = store.write_conn();

    let mut read_stmt = conn.prepare("SELECT attributes FROM nodes WHERE fqn = ?1")?;
    let mut write_stmt = conn.prepare("UPDATE nodes SET attributes = ?1 WHERE fqn = ?2")?;

    for (fqn, coverage_data) in updates {
        // Read current attributes
        let current_attrs: String =
            match read_stmt.query_row(rusqlite::params![fqn], |row| row.get::<_, String>(0)) {
                Ok(attrs) => attrs,
                Err(_) => continue, // Node not found, skip
            };

        // Parse current attributes JSON
        let mut attrs: serde_json::Value =
            serde_json::from_str(&current_attrs).unwrap_or_else(|_| serde_json::json!({}));

        // Insert coverage data under the "coverage" key
        if let Some(obj) = attrs.as_object_mut() {
            obj.insert(
                "coverage".to_string(),
                serde_json::to_value(coverage_data).unwrap_or(serde_json::Value::Null),
            );
        }

        // Write back updated attributes
        let attrs_str = serde_json::to_string(&attrs).unwrap_or_else(|_| "{}".to_string());
        write_stmt.execute(rusqlite::params![attrs_str, fqn])?;
    }

    Ok(())
}
