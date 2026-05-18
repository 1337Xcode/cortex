#![allow(dead_code)]

mod agents;
mod bundle;
mod cli;
mod config;
mod error;
mod indexer;
mod mcp;
mod memory;
mod security;
mod store;
mod telemetry;
mod version;
mod watcher;

use std::process;
use std::sync::Arc;

use clap::Parser;

use cli::{
    BundleCommand, Cli, Command, ConfigCommand, FederateCommand, HookCommand, MemoryCommand,
    QueryCommand, SecurityCommand, SemanticCommand,
};
use config::Config;
use store::db::StoreManager;
use version::VERSION;

fn main() {
    let cli = Cli::parse();

    // Load configuration.
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    // Initialize tracing subscriber.
    // In serve mode, logs must go to stderr - stdout is reserved for JSON-RPC.
    let is_serve = matches!(cli.command, Command::Serve { .. });
    telemetry::init_tracing(&config.log_level, is_serve);

    // Create data directory if needed.
    if let Err(e) = std::fs::create_dir_all(&config.data_dir) {
        eprintln!("error: failed to create data directory '{}': {e}", config.data_dir.display());
        process::exit(1);
    }

    // Open StoreManager.
    let store = match StoreManager::with_pool_size(&config.data_dir, config.pool_size) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    // Run embedded migrations (compiled into the binary - no external files needed).
    {
        let conn = store.write_conn();
        if let Err(e) = store::migrations::run_embedded_migrations(&conn) {
            eprintln!("error: failed to run migrations: {e}");
            process::exit(1);
        }
    }

    // Print startup line.
    // Print startup line to stderr (stdout is reserved for MCP JSON-RPC when serving).
    eprintln!(
        "cortex {} | repo: {} | data: {}",
        VERSION,
        config.repo_root.display(),
        config.data_dir.display()
    );

    // Wrap store in Arc for shared ownership.
    let store = Arc::new(store);

    // Dispatch subcommand.
    match cli.command {
        Command::Serve { smart_tools } => {
            if let Err(e) = cli::commands::serve::run_with_options(&config, Arc::clone(&store), smart_tools) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Index => {
            match indexer::pipeline::index_repository(&config.repo_root, &store) {
                Ok(stats) => {
                    println!(
                        "Index complete: {} scanned, {} indexed, {} skipped, {} deleted ({} ms)",
                        stats.files_scanned,
                        stats.files_indexed,
                        stats.files_skipped,
                        stats.files_deleted,
                        stats.duration_ms
                    );

                    // Auto-export bundle after indexing if enabled
                    if config.auto_bundle_export {
                        let output_dir = config.data_dir.clone();
                        if let Err(e) = bundle::export::export_bundle(&store, &output_dir) {
                            eprintln!("warning: auto bundle export failed: {e}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        }
        Command::IndexFile { path } => {
            if let Err(e) = cli::commands::index_file::run(&path, &config.repo_root) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Install { platform } => {
            if let Some(ref p) = platform {
                match crate::agents::detect::detect_installed_agents(&config.repo_root)
                    .into_iter()
                    .find(|a| a.name == *p || a.display_name.to_lowercase() == p.to_lowercase())
                {
                    Some(agent) => {
                        let binary = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("cortex"));
                        if let Err(e) = crate::agents::configure::configure_agent(&agent, &binary) {
                            eprintln!("error: {e}");
                            process::exit(1);
                        }
                        println!("Configured {} at {}", agent.display_name, agent.config_path.display());
                    }
                    None => {
                        eprintln!("Platform '{}' not detected. Supported: claude-code, cursor, windsurf, vscode, kiro, zed, jetbrains, cline, continue, aider", p);
                        process::exit(1);
                    }
                }
            } else if let Err(e) = cli::commands::install::run(&config) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Bundle(sub) => match sub {
            BundleCommand::Export { format } => {
                let output_dir = config.data_dir.clone();
                match format.as_str() {
                    "ccg" => {
                        match bundle::ccg::export_ccg(&store, &output_dir) {
                            Ok(stats) => {
                                println!(
                                    "CCG exported: {} nodes, {} edges -> {}",
                                    stats.nodes_exported,
                                    stats.edges_exported,
                                    stats.output_path,
                                );
                            }
                            Err(e) => {
                                eprintln!("error: {e}");
                                process::exit(1);
                            }
                        }
                    }
                    _ => {
                        if let Err(e) = cli::commands::bundle::run_export(&store, &output_dir) {
                            eprintln!("error: {e}");
                            process::exit(1);
                        }
                    }
                }
            }
            BundleCommand::Import { path } => {
                let bundle_path = path.unwrap_or_else(|| config.data_dir.join("cortex.json"));
                if let Err(e) = cli::commands::bundle::run_import(&store, &bundle_path) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Query(sub) => match sub {
            QueryCommand::Callers { fqn, depth } => {
                if let Err(e) = cli::commands::query::callers(&fqn, depth, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::Callees { fqn, depth } => {
                if let Err(e) = cli::commands::query::callees(&fqn, depth, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::Find {
                pattern,
                kind,
                limit,
            } => {
                if let Err(e) =
                    cli::commands::query::find(&pattern, kind.as_deref(), limit, &store)
                {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::Architecture => {
                if let Err(e) = cli::commands::query::architecture(&store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::DeadCode { limit } => {
                if let Err(e) = cli::commands::query::dead_code(limit, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::BlastRadius { fqn, depth } => {
                if let Err(e) = cli::commands::query::blast_radius(&fqn, depth, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            QueryCommand::Changes { since } => {
                if let Err(e) = cli::commands::query::changes(since, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Status => {
            let conn = store.read_conn();
            let arch = crate::store::queries::graph::get_architecture_summary(&conn).unwrap_or_else(|_| {
                crate::store::queries::graph::ArchitectureSummary {
                    total_nodes: 0,
                    total_edges: 0,
                    files_indexed: 0,
                    languages: vec![],
                    counts_by_kind: vec![],
                    top_level_modules: vec![],
                    entry_points: vec![],
                }
            });
            println!("Cortex status");
            println!("  Nodes: {}", arch.total_nodes);
            println!("  Edges: {}", arch.total_edges);
            println!("  Files: {}", arch.files_indexed);
            println!("  Languages: {}", arch.languages.join(", "));
        }
        Command::Memory(sub) => match sub {
            MemoryCommand::List => {
                if let Err(e) = cli::commands::memory_show::run(&store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            MemoryCommand::Prune { stale: _ } => {
                let conn = store.write_conn();
                match crate::store::queries::memory::prune_stale_observations(&conn, None) {
                    Ok(count) => println!("Pruned {} stale observations", count),
                    Err(e) => { eprintln!("error: {e}"); process::exit(1); }
                }
            }
            MemoryCommand::Show => {
                if let Err(e) = cli::commands::memory_show::run(&store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Security(sub) => match sub {
            SecurityCommand::Scan => {
                if let Err(e) = cli::commands::security::run_scan(&store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            SecurityCommand::Sbom => {
                if let Err(e) = cli::commands::security::run_sbom(&config, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            SecurityCommand::Vulns => {
                if let Err(e) = cli::commands::security::run_vulns(&config, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            SecurityCommand::Report => {
                if let Err(e) = cli::commands::security::run_report(&config, &store) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Config(sub) => match sub {
            ConfigCommand::Get { key } => {
                println!("Configuration key '{}' is managed via environment variables (CORTEX_{}) or .cortex/config.toml", key, key.to_uppercase());
            }
            ConfigCommand::Set { key, value } => {
                println!("To set '{}' = '{}', add it to .cortex/config.toml or set CORTEX_{}={}", key, value, key.to_uppercase(), value);
            }
            ConfigCommand::Reset => {
                let config_path = config.repo_root.join(".cortex").join("config.toml");
                if config_path.exists() {
                    std::fs::remove_file(&config_path).ok();
                    println!("Configuration reset (removed .cortex/config.toml)");
                } else {
                    println!("No config file to reset (.cortex/config.toml does not exist)");
                }
            }
        },
        Command::Semantic(sub) => match sub {
            SemanticCommand::Enable => {
                if let Err(e) = indexer::embedder::enable(&config.data_dir) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            SemanticCommand::Disable => {
                if let Err(e) = indexer::embedder::disable(&config.data_dir) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            SemanticCommand::Status => {
                println!("{}", indexer::embedder::status(&config.data_dir));
            }
        },
        Command::Report { output } => {
            let output_path = output.unwrap_or_else(|| config.repo_root.join("CORTEX_REPORT.md"));
            if let Err(e) = cli::commands::report::run(&store, &output_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Viz { export, port: _ } => {
            if let Some(path) = export {
                if let Err(e) = cli::commands::viz::export_html(&store, &path) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            } else {
                // Start the visualization server (delegates to existing visualizer)
                println!("{}", cli::commands::visualizer::run(true));
            }
        }
        Command::Clone { url, dir } => {
            if let Err(e) = cli::commands::clone::run(&url, dir.as_deref()) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Impact { fqn, depth } => {
            if let Err(e) = cli::commands::impact::run(&fqn, depth, &store) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Explain { fqn } => {
            if let Err(e) = cli::commands::explain::run(&fqn, &store) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Diff { base, head } => {
            if let Err(e) = cli::commands::diff::run(&base, &head, &store) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Ci { fail_on_taint, fail_on_dead_code_above, fail_on_owasp, format } => {
            match cli::commands::ci::run(&config, &store, fail_on_taint, fail_on_dead_code_above, fail_on_owasp, &format) {
                Ok(failed) => {
                    if failed {
                        process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        }
        Command::Hook(sub) => match sub {
            HookCommand::Install => {
                if let Err(e) = cli::commands::hook::install(&config.repo_root) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            HookCommand::Remove => {
                if let Err(e) = cli::commands::hook::remove(&config.repo_root) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            HookCommand::Status => {
                if let Err(e) = cli::commands::hook::status(&config.repo_root) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Modules { threshold, build_system } => {
            if build_system {
                let info = indexer::build_system::detect(&config.repo_root);
                println!("Build system: {:?}", info.build_system);
                if info.members.is_empty() {
                    println!("No workspace members detected.");
                } else {
                    println!("Workspace members ({}):", info.members.len());
                    for member in &info.members {
                        println!("  {} ({})", member.name, member.path);
                    }
                }
            } else if let Err(e) = cli::commands::modules::run(&store, threshold) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Watch => {
            if let Err(e) = cli::commands::watch::run() {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Coverage { lcov, limit } => {
            if let Err(e) = cli::commands::coverage::run(&store, &lcov, limit) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Hierarchy { fqn } => {
            if let Err(e) = cli::commands::hierarchy::run(&fqn, &store) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Hotspots { months, limit } => {
            if let Err(e) = cli::commands::hotspots::run(&config, &store, months, limit) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
        Command::Federate(sub) => match sub {
            FederateCommand::Add { path } => {
                if let Err(e) = cli::commands::federate::add(&config.data_dir, &path) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            FederateCommand::List => {
                if let Err(e) = cli::commands::federate::list(&config.data_dir) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
            FederateCommand::Remove { name } => {
                if let Err(e) = cli::commands::federate::remove(&config.data_dir, &name) {
                    eprintln!("error: {e}");
                    process::exit(1);
                }
            }
        },
        Command::Ingest { path, types } => {
            if let Err(e) = cli::commands::ingest::run(&store, &path, &types) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
    }
}
