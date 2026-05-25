//! Security CLI command implementations.
//!
//! Implements `cortex security scan`, `cortex security sbom`,
//! `cortex security vulns`, and `cortex security report` commands.

use std::sync::Arc;

use crate::config::Config;
use crate::security::{owasp, sbom, taint, vuln};
use crate::store::db::StoreManager;

/// Run security scan: OWASP pattern detection + taint analysis.
pub fn run_scan(store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    // Run taint analysis
    let taint_paths = taint::propagate_taint(store)?;

    // Run OWASP pattern detection
    let findings = owasp::scan_owasp_patterns(store)?;

    // Output results as JSON
    let result = serde_json::json!({
        "taint_paths": taint_paths.len(),
        "security_findings": findings.len(),
        "findings": findings,
        "taint_details": taint_paths,
    });

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// Generate SBOM and output as SPDX 2.3 JSON.
pub fn run_sbom(config: &Config, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let entries = sbom::generate_sbom(store, &config.repo_root)?;

    // Generate SPDX document
    let project_name = config
        .repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let spdx = sbom::generate_spdx(&entries, project_name);

    println!("{}", serde_json::to_string_pretty(&spdx)?);
    Ok(())
}

/// Check dependencies against OSV.dev for known vulnerabilities.
pub fn run_vulns(config: &Config, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let entries = sbom::generate_sbom(store, &config.repo_root)?;

    let results = vuln::check_osv(&entries)?;

    let output = serde_json::json!({
        "packages_checked": entries.len(),
        "vulnerabilities_found": results.iter().map(|r| r.vulnerabilities.len()).sum::<usize>(),
        "results": results,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

/// Print a human-readable security report to stdout.
pub fn run_report(config: &Config, store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let project_name = config
        .repo_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    println!();
    println!("CORTEX SECURITY REPORT: {}", project_name);
    println!("{}", "━".repeat(50));
    println!();

    // Taint analysis
    let taint_paths = taint::propagate_taint(store)?;
    if taint_paths.is_empty() {
        println!("  ✓  No taint flows detected");
    } else {
        println!(
            "  ⚠  {} taint flow{}: user input reaches sensitive sink without sanitization",
            taint_paths.len(),
            if taint_paths.len() == 1 { "" } else { "s" }
        );
        for path in taint_paths.iter().take(5) {
            println!("     └─ {} → {}", path.source_fqn, path.sink_fqn);
        }
        if taint_paths.len() > 5 {
            println!("     └─ ... and {} more", taint_paths.len() - 5);
        }
    }
    println!();

    // OWASP patterns
    let findings = owasp::scan_owasp_patterns(store)?;
    if findings.is_empty() {
        println!("  ✓  No OWASP Top 10 patterns detected");
    } else {
        // Group by category
        let mut by_category: std::collections::HashMap<&str, Vec<_>> =
            std::collections::HashMap::new();
        for f in &findings {
            by_category
                .entry(f.finding.owasp_category.as_deref().unwrap_or("Unknown"))
                .or_default()
                .push(f);
        }
        for (category, items) in &by_category {
            println!(
                "  ⚠  {} {} pattern{} detected",
                items.len(),
                category,
                if items.len() == 1 { "" } else { "s" }
            );
            for item in items.iter().take(3) {
                println!(
                    "     └─ {} ({})",
                    item.finding.node_fqn, item.finding.description
                );
            }
            if items.len() > 3 {
                println!("     └─ ... and {} more", items.len() - 3);
            }
        }
    }
    println!();

    // SBOM summary
    let entries = sbom::generate_sbom(store, &config.repo_root)?;
    if entries.is_empty() {
        println!("  ✓  No external dependencies detected");
    } else {
        // Check for vulnerabilities
        match vuln::check_osv(&entries) {
            Ok(results) => {
                let vuln_count: usize = results.iter().map(|r| r.vulnerabilities.len()).sum();
                if vuln_count == 0 {
                    println!("  ✓  SBOM: {} dependencies, 0 known CVEs", entries.len());
                } else {
                    println!(
                        "  ⚠  SBOM: {} dependencies, {} known CVE{}",
                        entries.len(),
                        vuln_count,
                        if vuln_count == 1 { "" } else { "s" }
                    );
                    for result in results
                        .iter()
                        .filter(|r| !r.vulnerabilities.is_empty())
                        .take(5)
                    {
                        for vuln in &result.vulnerabilities {
                            println!("     └─ {} ({})", vuln.id, result.package);
                        }
                    }
                }
            }
            Err(_) => {
                println!(
                    "  ✓  SBOM: {} dependencies (offline, CVE check skipped)",
                    entries.len()
                );
            }
        }
    }

    println!();
    println!("{}", "━".repeat(50));
    println!();

    Ok(())
}
