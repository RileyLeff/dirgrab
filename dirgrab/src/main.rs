// --- FILE: dirgrab/src/main.rs ---

use anyhow::{Context, Result};
use arboard::Clipboard;
use clap::Parser;
use dirgrab_lib::{grab_contents, GrabConfig};
use log::{debug, error, info, LevelFilter};
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Concatenates files from a directory, respecting Git context. Includes file headers by default.",
    long_about = "Dirgrab walks a directory, finds relevant files (using git ls-files if in a Git repo, otherwise walking the directory), applies exclusions, and concatenates their content to stdout, a file, or the clipboard.\n\nBy default, the content of each file is preceded by a '--- FILE: <filename> ---' header. Use --no-headers to disable this.\nBy default, 'dirgrab.txt' is excluded. Use --include-default-output to override this specific exclusion.\nUse --no-git to ignore Git context entirely and treat the target as a plain directory." // Updated long_about
)]
struct Cli {
    /// Optional path to the repository or directory to process.
    /// If not provided, the current working directory is used.
    #[arg(index = 1)]
    target_path: Option<PathBuf>,

    /// Write output to a file instead of stdout.
    #[arg(short = 'o', long, conflicts_with = "clipboard")]
    output: Option<PathBuf>,

    /// Copy output to the system clipboard instead of stdout or a file.
    #[arg(short = 'c', long, conflicts_with = "output")]
    clipboard: bool,

    /// Disable the default inclusion of '--- FILE: `<filename>` ---' headers.
    #[arg(long)]
    no_headers: bool,

    /// Add patterns to exclude files or directories. Can be used multiple times.
    /// Uses .gitignore glob syntax. Examples: -e "*.log" -e "target/"
    #[arg(short = 'e', long = "exclude", value_name = "PATTERN")]
    exclude_patterns: Vec<String>,

    /// [Git Mode Only] Include untracked files (still respects .gitignore and excludes).
    /// Has no effect if --no-git is used. // <-- Updated help text
    #[arg(short = 'u', long)]
    include_untracked: bool,

    /// Include the default output file ('dirgrab.txt') if it exists and isn't otherwise excluded.
    #[arg(long)]
    include_default_output: bool,

    // --- Start Modification (C) ---
    /// Ignore Git context and treat the target as a plain directory.
    /// This disables .gitignore processing and the effect of -u/--include-untracked.
    #[arg(long)]
    no_git: bool,
    // --- End Modification (C) ---
    /// Enable verbose output. Use -v for info, -vv for debug, -vvv for trace.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize Logging
    let log_level = match cli.verbose {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };
    env_logger::Builder::new().filter_level(log_level).init();

    info!("Log level set to: {}", log_level);
    debug!("Parsed arguments: {:?}", cli);

    // Determine Target Path
    let target_path = match cli.target_path {
        Some(path) => path,
        None => std::env::current_dir().context("Failed to get current working directory")?,
    };
    info!("Target path determined as: {:?}", target_path);

    // Determine Header Inclusion
    let add_headers = !cli.no_headers;
    if add_headers {
        info!("File headers will be included (default).");
    } else {
        info!("File headers will be excluded (--no-headers specified).");
    }

    // Create Library Config
    let config = GrabConfig {
        target_path,
        add_headers,
        exclude_patterns: cli.exclude_patterns,
        include_untracked: cli.include_untracked,
        include_default_output: cli.include_default_output,
        // --- Start Modification (C) ---
        no_git: cli.no_git, // Pass the new flag
                            // --- End Modification (C) ---
    };

    // Call Library
    let combined_content = match grab_contents(&config) {
        Ok(content) => content,
        Err(e) => {
            error!("Error during dirgrab operation: {}", e);
            return Err(e.into());
        }
    };

    if combined_content.is_empty() {
        info!("No content was generated (likely no files matched or files were empty/binary).");
        return Ok(());
    }

    // Handle Output
    if cli.clipboard {
        info!("Copying output to clipboard...");
        let mut clipboard = Clipboard::new().context("Failed to initialize clipboard")?;
        clipboard
            .set_text(combined_content)
            .context("Failed to copy content to clipboard")?;
        info!("Successfully copied content to clipboard.");
    } else if let Some(output_path) = cli.output {
        info!("Writing output to file: {:?}", output_path);
        let mut file = File::create(&output_path)
            .with_context(|| format!("Failed to create output file: {:?}", output_path))?;
        file.write_all(combined_content.as_bytes())
            .with_context(|| format!("Failed to write content to file: {:?}", output_path))?;
        info!("Successfully wrote content to {:?}", output_path);
    } else {
        // Default to stdout
        debug!("Writing output to stdout...");
        io::stdout()
            .write_all(combined_content.as_bytes())
            .context("Failed to write content to stdout")?;
        io::stdout().flush().context("Failed to flush stdout")?;
        debug!("Finished writing to stdout.");
    }

    Ok(())
}
