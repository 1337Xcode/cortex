//! Coverage command: cross-reference call graph with test coverage data.
//!
//! `cortex coverage --lcov coverage.lcov` reads LCOV/gcov output and ranks
//! untested functions by how many other functions call them. The most-called
//! untested function is your highest-risk coverage gap.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use crate::store::db::StoreManager;

/// A single coverage gap entry for display.
struct CoverageGap {
    fqn: String,
    file: String,
    start_line: u32,
    caller_count: u32,
    status: String,
}

/// Parse an LCOV file and return a map of file -> set of uncovered line numbers.
///
/// LCOV format:
///   SF:<source file path>
///   DA:<line number>,<execution count>
///   end_of_record
fn parse_lcov(path: &Path) -> Result<HashMap<String, HashSet<u32>>, anyhow::Error> {
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

    let mut file_coverage: HashMap<String, HashSet<u32>> = HashMap::new();
    let mut current_file: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        if let Some(sf) = line.strip_prefix("SF:") {
            current_file = Some(sf.to_string());
        } else if line.starts_with("DA:") {
            if let Some(ref file) = current_file {
                // DA:line_number,execution_count
                let parts: Vec<&str> = line[3..].splitn(2, ',').collect();
                if parts.len() == 2 {
                    if let (Ok(line_num), Ok(count)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u64>())
                    {
                        if count == 0 {
                            file_coverage
                                .entry(file.clone())
                                .or_default()
                                .insert(line_num);
                        }
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
    let uncovered_lines = parse_lcov(lcov_path)?;

    if uncovered_lines.is_empty() {
        println!("No uncovered lines found in LCOV data. All lines appear covered.");
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

    // For each node, check if any of its lines are uncovered
    let mut gaps: Vec<CoverageGap> = Vec::new();

    for (fqn, _kind, file, start_line, end_line) in &nodes {
        let norm_file = normalize_path(file);

        // Check if this file has any uncovered lines in the LCOV data
        let uncovered = uncovered_lines.iter().find_map(|(lcov_file, lines)| {
            let norm_lcov = normalize_path(lcov_file);
            if norm_lcov == norm_file || norm_lcov.ends_with(&norm_file) || norm_file.ends_with(&norm_lcov) {
                Some(lines)
            } else {
                None
            }
        });

        if let Some(uncovered_set) = uncovered {
            // Check if any line in the node's range is uncovered
            let has_uncovered = (*start_line..=*end_line).any(|l| uncovered_set.contains(&l));

            if has_uncovered {
                let covered_count = (*start_line..=*end_line)
                    .filter(|l| !uncovered_set.contains(l))
                    .count();
                let total_lines = (end_line - start_line + 1) as usize;
                let status = if covered_count == 0 {
                    "UNCOVERED".to_string()
                } else {
                    format!("PARTIAL ({}/{})", covered_count, total_lines)
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

    if gaps.is_empty() {
        println!("All indexed functions have full test coverage. No gaps found.");
        return Ok(());
    }

    // Count callers for each gap node
    let mut caller_counts: HashMap<String, u32> = HashMap::new();
    let mut count_stmt = conn.prepare(
        "SELECT target_fqn, COUNT(*) FROM edges WHERE kind = 'Calls' GROUP BY target_fqn",
    )?;

    let rows = count_stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
    })?;

    for row in rows {
        if let Ok((fqn, count)) = row {
            caller_counts.insert(fqn, count);
        }
    }

    // Assign caller counts to gaps
    for gap in &mut gaps {
        gap.caller_count = caller_counts.get(&gap.fqn).copied().unwrap_or(0);
    }

    // Sort by caller count descending (highest risk first)
    gaps.sort_by(|a, b| b.caller_count.cmp(&a.caller_count));

    // Limit results
    gaps.truncate(limit);

    // Print results
    println!();
    println!("COVERAGE GAPS (ranked by risk)");
    println!("{}", "━".repeat(90));
    println!();
    println!(
        "  {:<40} {:<25} {:>5} {:>8} {}",
        "Function", "File", "Line", "Callers", "Coverage"
    );
    println!(
        "  {:<40} {:<25} {:>5} {:>8} {}",
        "────────", "────", "────", "───────", "────────"
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
    println!();

    Ok(())
}
