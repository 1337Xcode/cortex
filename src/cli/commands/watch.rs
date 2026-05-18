//! Watch command: informs users that file watching is integrated into `cortex serve`.

/// Run the watch command.
pub fn run() -> Result<(), anyhow::Error> {
    println!("The file watcher runs automatically during `cortex serve`.");
    println!("To see indexing activity, check the logs (CORTEX_LOG_LEVEL=info cortex serve).");
    println!();
    println!("For foreground watching with visible output, use:");
    println!("  CORTEX_LOG_LEVEL=info cortex serve");
    Ok(())
}
