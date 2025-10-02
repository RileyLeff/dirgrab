// --- FILE: dirgrab/src/main.rs ---

use anyhow::{Context, Result};
use arboard::Clipboard;
use clap::Parser;
use config_loader::{build_run_settings, StatsSettings};
use dirgrab_lib::{grab_contents, GrabConfig};
use log::{debug, error, info, LevelFilter};
use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

mod config_loader;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    // Updated about and long_about for new PDF default
    about = "Concatenates files from a directory, respecting Git context. Includes file headers, directory tree, and PDF text extraction by default.",
    long_about = "Dirgrab walks a directory, finds relevant files (using git ls-files if in a Git repo, otherwise walking the directory), applies exclusions, and concatenates their content to stdout, a file, or the clipboard.\n\nBy default, text content is extracted from PDF files. Use --no-pdf to disable this.\nBy default, a directory structure overview is prepended. Use --no-tree to disable this.\nBy default, the content of each file is preceded by a '--- FILE: <filename> ---' header. Use --no-headers to disable this.\nBy default, 'dirgrab.txt' is excluded. Use --include-default-output to override this specific exclusion.\nUse --no-git to ignore Git context entirely and treat the target as a plain directory.\n\nUse -s or --stats to print output size and word count to stderr upon completion."
)]
pub(crate) struct Cli {
    /// Optional path to the repository or directory to process.
    /// If not provided, the current working directory is used.
    #[arg(index = 1)]
    target_path: Option<PathBuf>,

    /// Write output to a file instead of stdout.
    /// If the flag is provided without a filename (e.g., `-o`), defaults to 'dirgrab.txt'.
    #[arg(
        short = 'o',
        long,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "dirgrab.txt",
        conflicts_with = "clipboard"
    )]
    output: Option<PathBuf>,

    /// Copy output to the system clipboard instead of stdout or a file.
    #[arg(short = 'c', long, conflicts_with = "output")]
    clipboard: bool,

    /// Disable the default inclusion of '--- FILE: `<filename>` ---' headers.
    #[arg(long)]
    no_headers: bool,

    /// Disable the default inclusion of the directory structure overview.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    no_tree: bool,

    /// Disable the default extraction of text content from PDF files.
    #[arg(long, action = clap::ArgAction::SetTrue)] // New flag to disable PDF extraction
    no_pdf: bool,

    /// Add patterns to exclude files or directories. Can be used multiple times.
    /// Uses .gitignore glob syntax. Examples: -e "*.log" -e "target/"
    #[arg(short = 'e', long = "exclude", value_name = "PATTERN")]
    exclude_patterns: Vec<String>,

    /// Include the default output file ('dirgrab.txt') if it exists and isn't otherwise excluded.
    #[arg(long)]
    include_default_output: bool,

    /// Ignore Git context and treat the target as a plain directory.
    /// This disables .gitignore processing and the effect of -u/--include-untracked.
    #[arg(long)]
    no_git: bool,

    /// Limit Git mode to tracked files only.
    #[arg(long)]
    tracked_only: bool,

    /// Operate on the entire repository even if TARGET_PATH is a subdirectory.
    #[arg(long)]
    all_repo: bool,

    /// Print output size (bytes) and word count to stderr upon completion.
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    print_stats: bool,

    /// Disable loading of global/local configuration files.
    #[arg(long)]
    no_config: bool,

    /// Provide an explicit configuration file to load (processed after global/local files).
    #[arg(long = "config", value_name = "FILE", value_hint = clap::ValueHint::FilePath)]
    config_path: Option<PathBuf>,

    /// Token ratio override for approximate token counting used with --stats.
    #[arg(long = "token-ratio", value_name = "FLOAT")]
    token_ratio: Option<f64>,

    /// Exclude the directory tree section when estimating tokens.
    #[arg(long = "tokens-exclude-tree")]
    tokens_exclude_tree: bool,

    /// Exclude file headers when estimating tokens.
    #[arg(long = "tokens-exclude-headers")]
    tokens_exclude_headers: bool,

    /// Legacy flag to force including untracked files (now default). Hidden for compatibility.
    #[arg(
        short = 'u',
        long = "include-untracked",
        action = clap::ArgAction::SetTrue,
        hide = true
    )]
    include_untracked_flag: bool,

    // REMOVED: convert_pdf flag
    // /// Optionally extract text content from PDF files using pdf-extract.
    // #[arg(long, action = clap::ArgAction::SetTrue)]
    // convert_pdf: bool,
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
    let target_path = match &cli.target_path {
        Some(path) => path.clone(),
        None => std::env::current_dir().context("Failed to get current working directory")?,
    };
    info!("Target path determined as: {:?}", target_path);

    let run_settings = build_run_settings(&cli, &target_path)?;
    let config = run_settings.grab_config;
    let stats_settings = run_settings.stats;

    if config.add_headers {
        info!("File headers will be included.");
    } else {
        info!("File headers will be excluded.");
    }

    if config.include_tree {
        info!("Directory tree will be included.");
    } else {
        info!("Directory tree will be excluded.");
    }

    if config.convert_pdf {
        info!("PDF text extraction will be attempted.");
    } else {
        info!("PDF text extraction is disabled.");
    }

    if config.no_git {
        info!("Operating in plain directory mode (--no-git).");
    } else if config.include_untracked {
        info!("Git mode will include untracked files by default.");
    } else {
        info!("Git mode limited to tracked files (tracked-only).");
    }

    if config.all_repo {
        info!("Git scope set to entire repository (--all-repo).");
    }

    // Call Library
    let combined_content = match grab_contents(&config) {
        Ok(content) => content,
        Err(e) => {
            error!("Error during dirgrab operation: {}", e);
            return Err(e.into());
        }
    };

    // Check if content is empty *after* potential tree generation
    if combined_content.is_empty() {
        info!("No content was generated.");
        // Print stats even if empty, but only if requested
        if stats_settings.enabled {
            eprintln!("Output Size: 0 bytes, 0 words");
        }
        return Ok(());
    }

    // Handle Output
    let output_destination = if cli.clipboard {
        info!("Copying output to clipboard...");
        let mut clipboard = Clipboard::new().context("Failed to initialize clipboard")?;
        clipboard
            .set_text(&combined_content)
            .context("Failed to copy content to clipboard")?;
        info!("Successfully copied content to clipboard.");
        "Clipboard".to_string()
    } else if let Some(ref output_path) = cli.output {
        info!("Writing output to file: {:?}", output_path);
        let mut file = File::create(output_path)
            .with_context(|| format!("Failed to create output file: {:?}", output_path))?;
        file.write_all(combined_content.as_bytes())
            .with_context(|| format!("Failed to write content to file: {:?}", output_path))?;
        info!("Successfully wrote content to {:?}", output_path);
        format!("File ({})", output_path.display())
    } else {
        // Default to stdout
        debug!("Writing output to stdout...");
        io::stdout()
            .write_all(combined_content.as_bytes())
            .context("Failed to write content to stdout")?;
        io::stdout().flush().context("Failed to flush stdout")?;
        debug!("Finished writing to stdout.");
        "stdout".to_string()
    };

    // Calculate and print stats to stderr *only if requested*
    if stats_settings.enabled {
        let byte_count = combined_content.len();
        // Simple word count based on whitespace splitting
        let word_count = combined_content.split_whitespace().count();
        let token_basis = build_token_basis(&combined_content, &config, &stats_settings);
        let char_count = token_basis.chars().count();
        let approx_tokens = if char_count == 0 {
            0
        } else {
            (char_count as f64 / stats_settings.token_ratio).ceil() as usize
        };
        let ratio_display = format_ratio(stats_settings.token_ratio);
        eprintln!(
            "Output Size (to {}): {} bytes, {} words, tokens≈{} (ratio={})",
            output_destination, byte_count, word_count, approx_tokens, ratio_display
        );
    }

    Ok(())
}

fn build_token_basis<'a>(
    full_output: &'a str,
    config: &GrabConfig,
    stats: &StatsSettings,
) -> Cow<'a, str> {
    let mut current = Cow::Borrowed(full_output);

    if stats.exclude_tree && config.include_tree {
        let trimmed = strip_tree_section(current.as_ref());
        current = Cow::Owned(trimmed);
    }

    if stats.exclude_headers && config.add_headers {
        let without_headers = strip_header_lines(current.as_ref());
        current = Cow::Owned(without_headers);
    }

    current
}

fn strip_tree_section(content: &str) -> String {
    const FILE_CONTENTS_HEADER: &str = "---\nFILE CONTENTS\n---\n\n";
    if let Some(idx) = content.find(FILE_CONTENTS_HEADER) {
        content[idx..].to_string()
    } else {
        String::new()
    }
}

fn strip_header_lines(content: &str) -> String {
    content
        .split_inclusive('\n')
        .filter(|chunk| {
            let line = chunk.trim_end_matches('\n');
            !line.starts_with("--- FILE: ")
        })
        .collect()
}

fn format_ratio(ratio: f64) -> String {
    let mut s = format!("{:.3}", ratio);
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s.is_empty() {
        "0".to_string()
    } else {
        s
    }
}

#[cfg(test)]
impl Cli {
    fn test_default() -> Self {
        Self {
            target_path: None,
            output: None,
            clipboard: false,
            no_headers: false,
            no_tree: false,
            no_pdf: false,
            exclude_patterns: Vec::new(),
            include_default_output: false,
            no_git: false,
            tracked_only: false,
            all_repo: false,
            print_stats: false,
            no_config: false,
            config_path: None,
            token_ratio: None,
            tokens_exclude_tree: false,
            tokens_exclude_headers: false,
            include_untracked_flag: false,
            verbose: 0,
        }
    }
}
