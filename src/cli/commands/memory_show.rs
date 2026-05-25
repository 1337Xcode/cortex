//! Memory show command: display all cross-session observations and ADRs.
//!
//! `cortex memory show` lists what agents have learned about the codebase
//! across sessions. Shows observations grouped by symbol, with staleness
//! indicators and timestamps.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::store::db::StoreManager;

/// Format a Unix timestamp as a human-readable date string.
fn format_timestamp(ts: i64) -> String {
    // Simple formatting: days ago relative to now
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let diff = now - ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Run the memory show command: list all observations and ADRs.
pub fn run(store: &Arc<StoreManager>) -> Result<(), anyhow::Error> {
    let conn = store.read_conn();

    // Query all observations
    let mut obs_stmt = conn.prepare(
        "SELECT id, node_fqn, observation_text, agent_id, node_hash_at_write, written_at, status, stale_reason \
         FROM observations ORDER BY node_fqn, written_at DESC",
    )?;

    #[allow(clippy::type_complexity)]
    let observations: Vec<(
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        Option<String>,
    )> = obs_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    // Query all ADRs
    let mut adr_stmt = conn.prepare(
        "SELECT id, title, body, status, linked_fqn, created_at, updated_at \
         FROM architectural_decisions ORDER BY created_at DESC",
    )?;

    #[allow(clippy::type_complexity)]
    let adrs: Vec<(String, String, String, String, Option<String>, i64, i64)> = adr_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();

    if observations.is_empty() && adrs.is_empty() {
        println!("No observations or ADRs stored yet.");
        println!();
        println!("Agents can store observations using the `write_observation` MCP tool,");
        println!("and architectural decisions using `write_adr`.");
        return Ok(());
    }

    // Print observations grouped by node FQN
    if !observations.is_empty() {
        println!();
        println!("OBSERVATIONS ({} total)", observations.len());
        println!("{}", "━".repeat(60));

        // Group by node_fqn
        #[allow(clippy::type_complexity)]
        let mut grouped: BTreeMap<
            &str,
            Vec<&(
                String,
                String,
                String,
                String,
                String,
                i64,
                String,
                Option<String>,
            )>,
        > = BTreeMap::new();
        for obs in &observations {
            grouped.entry(obs.1.as_str()).or_default().push(obs);
        }

        for (fqn, obs_list) in &grouped {
            println!();
            println!("  {}", fqn);
            for obs in obs_list {
                let status_marker = match obs.6.as_str() {
                    "stale" => " [STALE]",
                    "archived" => " [ARCHIVED]",
                    _ => "",
                };
                let time_str = format_timestamp(obs.5);
                println!(
                    "    {} {}{} (by {}, {})",
                    if obs.6 == "stale" { "○" } else { "●" },
                    obs.2,
                    status_marker,
                    obs.3,
                    time_str
                );
                if let Some(ref reason) = obs.7 {
                    println!("      Stale reason: {}", reason);
                }
            }
        }
    }

    // Print ADRs
    if !adrs.is_empty() {
        println!();
        println!();
        println!("ARCHITECTURAL DECISIONS ({} total)", adrs.len());
        println!("{}", "━".repeat(60));

        for adr in &adrs {
            let time_str = format_timestamp(adr.5);
            let linked = adr
                .4
                .as_deref()
                .map(|f| format!(" -> {}", f))
                .unwrap_or_default();

            println!();
            println!("  [{}] {}{}", adr.3.to_uppercase(), adr.1, linked);
            // Print first line of body as summary
            let summary: &str = adr.2.lines().next().unwrap_or(&adr.2);
            if summary.len() > 70 {
                println!("    {}..", &summary[..68]);
            } else {
                println!("    {}", summary);
            }
            println!("    Created: {}", time_str);
        }
    }

    println!();
    println!("{}", "━".repeat(60));

    // Summary
    let active_count = observations.iter().filter(|o| o.6 == "active").count();
    let stale_count = observations.iter().filter(|o| o.6 == "stale").count();
    let archived_count = observations.iter().filter(|o| o.6 == "archived").count();

    println!();
    println!(
        "  Observations: {} active, {} stale, {} archived",
        active_count, stale_count, archived_count
    );
    println!("  ADRs: {}", adrs.len());

    if stale_count > 0 {
        println!();
        println!("  Tip: run `cortex memory prune --stale` to archive stale observations.");
    }

    println!();

    Ok(())
}
