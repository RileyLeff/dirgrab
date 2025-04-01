use anyhow::{Context, Result}; // Use anyhow for easy error handling in the binary
use arboard::Clipboard;
use clap::Parser;
use dirgrab_lib::{grab_contents, GrabConfig}; // Import from our library
use log::{debug, error, info, LevelFilter}; // Use log levels
use std::fs::File;
use std::io::{self, Write}; // Use io::stdout for writing
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Concatenates files from a directory, respecting Git context. Includes file headers by default.",
    long_about = "Dirgrab walks a directory, finds relevant files (using git ls-files if in a Git repo, otherwise walking the directory), applies exclusions, and concatenates their content to stdout, a file, or the clipboard.\n\nBy default, the content of each file is preceded by a '--- FILE: <filename> ---' header. Use --no-headers to disable this."
)]
struct Cli {
    /// Optional path to the repository or directory to process.
    /// If not provided, the current working directory is used.
    #[arg(index = 1)] // Make it a positional argument
    target_path: Option<PathBuf>,

    /// Write output to a file instead of stdout.
    #[arg(short = 'o', long, conflicts_with = "clipboard")]
    output: Option<PathBuf>,

    /// Copy output to the system clipboard instead of stdout or a file.
    #[arg(short = 'c', long, conflicts_with = "output")]
    clipboard: bool,

    /// Disable the default inclusion of '--- FILE: `<filename>` ---' headers.
    #[arg(long)] // No short flag needed for disabling a default typically
    no_headers: bool,

    // Note: We removed the original '-H, --headers' flag as it's now the default.
    // If someone uses `-H`, clap will likely give an unknown argument error,
    // which is acceptable. Alternatively, we could keep -H as a hidden/deprecated alias
    // that does nothing, but simply removing it is cleaner.
    /// Add patterns to exclude files or directories. Can be used multiple times.
    /// Uses .gitignore glob syntax. Examples: -e "*.log" -e "target/"
    /// In Git mode, these are added to 'git ls-files'.
    /// In non-Git mode, these are applied during directory walk.
    #[arg(short = 'e', long = "exclude", value_name = "PATTERN")]
    exclude_patterns: Vec<String>,

    /// [Git Mode Only] Include untracked files (still respects .gitignore and excludes).
    #[arg(short = 'u', long)]
    include_untracked: bool,

    /// Enable verbose output. Use -v for info, -vv for debug, -vvv for trace.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // --- Initialize Logging ---
    let log_level = match cli.verbose {
        0 => LevelFilter::Warn,  // Default: Show warnings and errors
        1 => LevelFilter::Info,  // -v: Show info, warnings, errors
        2 => LevelFilter::Debug, // -vv: Show debug, info, warnings, errors
        _ => LevelFilter::Trace, // -vvv and more: Show everything
    };

    env_logger::Builder::new()
        .filter_level(log_level) // Set log level based on -v flags
        .init();

    info!("Log level set to: {}", log_level);
    debug!("Parsed arguments: {:?}", cli);

    // --- Determine Target Path ---
    let target_path = match cli.target_path {
        Some(path) => path,
        None => std::env::current_dir().context("Failed to get current working directory")?,
    };
    info!("Target path determined as: {:?}", target_path);

    // --- Determine Header Inclusion ---
    // Headers are included unless --no-headers is specified.
    let add_headers = !cli.no_headers;
    if add_headers {
        info!("File headers will be included (default).");
    } else {
        info!("File headers will be excluded (--no-headers specified).");
    }

    // --- Create Library Config ---
    let config = GrabConfig {
        target_path,                            // Use the determined path
        add_headers,                            // Use the calculated value based on --no-headers
        exclude_patterns: cli.exclude_patterns, // Pass exclusions from CLI
        include_untracked: cli.include_untracked,
    };

    // --- Call Library ---
    let combined_content = match grab_contents(&config) {
        Ok(content) => content,
        Err(e) => {
            error!("Error during dirgrab operation: {}", e);
            return Err(e.into()); // Convert GrabError to anyhow::Error
        }
    };

    // Check if content is empty and inform user if so (library already warns)
    if combined_content.is_empty() {
        info!("No content was generated (likely no files matched or files were empty/binary).");
        return Ok(());
    }

    // --- Handle Output ---
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

    Ok(()) // Indicate successful execution
}
