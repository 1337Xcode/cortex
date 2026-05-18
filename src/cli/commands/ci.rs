//! CI command: structured output with exit codes for quality gates.
//!
//! `cortex ci --fail-on-taint --fail-on-owasp --fail-on-dead-code-above 15`
//! runs analysis and exits non-zero if thresholds are exceeded.
//! Designed for CI pipelines (GitHub Actions, GitLab CI, etc.)

use std::sync::Arc;

use crate::config::Config;
use crate::security::{owasp, sbom, taint, vuln};
use crate::store::db::StoreManager;
use crate::store::queries::graph;

/// CI check result.
struct CiResult {
    taint_flows: usize,
    owasp_findings: usize,
    dead_code_count: usize,
    total_nodes: usize,
    dead_code_pct: f64,
    dependency_vulns: usize,
    dependencies_total: usize,
}

/// Run CI checks and exit with appropriate code.
pub fn run(
    config: &Config,
    store: &Arc<StoreManager>,
    fail_on_taint: bool,
    fail_on_dead_code_above: Option<f64>,
    fail_on_owasp: bool,
    format: &str,
) -> Result<bool, anyhow::Error> {
    let conn = store.read_conn();

    // Run all analyses
    let taint_paths = taint::propagate_taint(store)?;
    let owasp_findings = owasp::scan_owasp_patterns(store)?;
    let dead_code = graph::find_dead_code(&conn, 10000)?;
    let arch = graph::get_architecture_summary(&conn)?;

    let dead_code_pct = if arch.total_nodes > 0 {
        (dead_code.len() as f64 / arch.total_nodes as f64) * 100.0
    } else {
        0.0
    };

    // SBOM + vulns
    let entries = sbom::generate_sbom(store, &config.repo_root)?;
    let vuln_count = match vuln::check_osv(&entries) {
        Ok(results) => results.iter().map(|r| r.vulnerabilities.len()).sum(),
        Err(_) => 0,
    };

    let result = CiResult {
        taint_flows: taint_paths.len(),
        owasp_findings: owasp_findings.len(),
        dead_code_count: dead_code.len(),
        total_nodes: arch.total_nodes,
        dead_code_pct,
        dependency_vulns: vuln_count,
        dependencies_total: entries.len(),
    };

    // Output
    if format == "json" {
        let json = serde_json::json!({
            "taint_flows": result.taint_flows,
            "owasp_findings": result.owasp_findings,
            "dead_code_count": result.dead_code_count,
            "dead_code_pct": result.dead_code_pct,
            "total_nodes": result.total_nodes,
            "dependency_vulns": result.dependency_vulns,
            "dependencies_total": result.dependencies_total,
        });
        println!("{}", serde_json::to_string_pretty(&json)?);
    } else {
        println!();
        println!("CORTEX CI — {}", config.repo_root.file_name().and_then(|n| n.to_str()).unwrap_or("project"));
        println!("{}", "━".repeat(50));
        println!();
        println!("  Taint flows:       {}", result.taint_flows);
        println!("  OWASP patterns:    {}", result.owasp_findings);
        println!("  Dead code:         {} / {} ({:.1}%)", result.dead_code_count, result.total_nodes, result.dead_code_pct);
        println!("  Dependencies:      {} ({} known CVEs)", result.dependencies_total, result.dependency_vulns);
        println!();
    }

    // Check thresholds
    let mut failed = false;

    if fail_on_taint && result.taint_flows > 0 {
        if format != "json" {
            eprintln!("  FAIL: {} taint flow(s) detected", result.taint_flows);
        }
        failed = true;
    }

    if fail_on_owasp && result.owasp_findings > 0 {
        if format != "json" {
            eprintln!("  FAIL: {} OWASP pattern(s) detected", result.owasp_findings);
        }
        failed = true;
    }

    if let Some(threshold) = fail_on_dead_code_above {
        if result.dead_code_pct > threshold {
            if format != "json" {
                eprintln!("  FAIL: dead code {:.1}% exceeds threshold {:.1}%", result.dead_code_pct, threshold);
            }
            failed = true;
        }
    }

    if format != "json" {
        if failed {
            println!("  Result: FAILED");
        } else {
            println!("  Result: PASSED");
        }
        println!();
        println!("{}", "━".repeat(50));
    }

    Ok(failed)
}
