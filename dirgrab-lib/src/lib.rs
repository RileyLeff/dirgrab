#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../Readme.md"))]

use ignore::gitignore::GitignoreBuilder;
use ignore::Match;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use thiserror::Error;
use walkdir::WalkDir;

// Re-export log macros for convenience if used internally
use log::{debug, error, info, warn};

// --- Public Configuration Struct ---

/// Configuration for the dirgrab operation.
///
/// This struct holds all the settings needed to control how `dirgrab`
/// finds and processes files within the specified target directory.
/// It is typically constructed by the calling application (e.g., the CLI)
/// based on user input.
#[derive(Debug, Clone)]
pub struct GrabConfig {
    /// The starting path for the operation (directory or Git repository).
    /// `dirgrab` will operate within this path. It will be canonicalized internally.
    pub target_path: PathBuf,

    /// If true, adds `'--- FILE: <filename> ---'` headers before each file's content
    /// in the final output string. The filename displayed will be relative to the
    /// Git repository root (if applicable) or the target path.
    pub add_headers: bool,

    /// A list of glob patterns (using .gitignore syntax) to exclude files or directories.
    /// These patterns are applied *in addition* to any `.gitignore` rules if operating
    /// in Git mode.
    /// In Git mode, they are passed to `git ls-files` as `:!<pattern>` pathspecs.
    /// In non-Git mode, they are used to filter the results from walking the directory.
    pub exclude_patterns: Vec<String>,

    /// If operating in Git mode, set this to true to include untracked files
    /// (files present in the working directory but not added to the index).
    /// This still respects `.gitignore` and the `exclude_patterns`.
    /// This setting has no effect if the `target_path` is not part of a Git repository.
    pub include_untracked: bool,
}

// --- Public Error Enum ---

/// Errors that can occur during the `dirgrab` library operations.
///
/// These errors cover issues ranging from file system access problems
/// to Git command failures and configuration errors.
#[derive(Error, Debug)]
pub enum GrabError {
    /// The initial `target_path` provided in the `GrabConfig` was not found
    /// on the filesystem or was inaccessible due to permissions.
    #[error("Target path not found or not accessible: {0}")]
    TargetPathNotFound(PathBuf),

    /// An I/O error occurred while accessing a path during the operation
    /// (e.g., reading a file, canonicalizing a path).
    #[error("IO error accessing path '{path}': {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A `git` command (like `git ls-files` or `git rev-parse`) failed to execute
    /// successfully, indicated by a non-zero exit status.
    /// Contains the command string, stderr, and stdout output for debugging.
    #[error("Failed to execute git command: {command:?}\n  stderr: {stderr}\n  stdout: {stdout}")]
    GitCommandError {
        command: String,
        stderr: String,
        stdout: String,
    },

    /// An error occurred while trying to spawn or run the `git` process itself.
    /// This commonly happens if `git` is not installed or not found in the system's PATH,
    /// but can also indicate permission errors preventing execution.
    #[error("Failed to run git command '{command}': {source}")]
    GitExecutionError {
        command: String,
        #[source]
        source: io::Error,
    },

    /// A file identified for processing could not be read as valid UTF-8 data.
    /// This usually indicates a binary file. In the default implementation of
    /// `grab_contents`, such files are logged as a warning and skipped, rather
    /// than returning this error directly.
    #[error("Failed to read non-UTF8 file: {0}")]
    NonUtf8File(PathBuf),

    /// Although `detect_git_repo` attempts to handle cases where a path is not
    /// in a repository gracefully (by returning `Ok(None)`), this error might
    /// occur if an unexpected issue prevents determining the root definitively.
    /// (Note: Current implementation less likely to return this specific variant).
    #[error("Could not determine repository root for: {0}")]
    RepoRootNotFound(PathBuf),

    /// Failed to build the glob pattern matcher from the patterns provided
    /// in `GrabConfig::exclude_patterns`. This might happen if a pattern has
    /// invalid syntax according to the `ignore` crate.
    #[error("Failed to build glob pattern matcher: {0}")]
    GlobMatcherBuildError(#[source] ignore::Error),

    /// An error occurred during directory traversal when operating in non-Git mode,
    /// likely related to permissions or accessing a specific directory entry.
    /// The default behavior logs a warning and skips the problematic entry.
    #[error("Error walking directory {path_display}: {source_str}")]
    WalkdirError {
        path_display: String, // Displayable path near the error
        source_str: String,   // The underlying error message from walkdir
    },
}

/// A convenience type alias for `Result<T, GrabError>`.
pub type GrabResult<T> = Result<T, GrabError>;

// --- Main Public Function ---

/// Performs the main `dirgrab` operation based on the provided configuration.
///
/// This function serves as the primary entry point into the `dirgrab-lib` core logic.
/// It reads files from the specified target directory, intelligently determining
/// whether to use Git context (`git ls-files`) or standard directory walking.
///
/// It applies exclusion patterns (`.gitignore` implicitly in Git mode, plus explicit
/// patterns from `GrabConfig`), concatenates the UTF-8 content of the selected files,
/// and optionally adds headers between file contents.
///
/// Non-UTF8/binary files encountered during processing are skipped with a warning message
/// logged via the `log` crate (level `WARN`). File system errors during reading
/// individual files are also typically logged as warnings, allowing the process to
/// continue with other files. More critical errors (like inability to run `git`,
/// invalid target path, or pattern compilation issues) will result in an `Err` return.
///
/// # Arguments
///
/// * `config`: A reference to a [`GrabConfig`] struct containing the parameters for
///   the operation, such as the target path, exclusion rules, and header preferences.
///
/// # Returns
///
/// * `Ok(String)`: A single `String` containing the concatenated UTF-8 content of
///   all selected and successfully read files. If no files are selected or readable,
///   this will be an empty string.
/// * `Err(GrabError)`: An error occurred that prevented the operation from completing
///   successfully. See [`GrabError`] for possible variants.
///
/// # Errors
///
/// This function can return various [`GrabError`] variants, including:
/// * [`GrabError::TargetPathNotFound`]: If the starting path doesn't exist or is inaccessible.
/// * [`GrabError::IoError`]: For general I/O issues (e.g., canonicalization).
/// * [`GrabError::GitCommandError`]: If a `git` command fails unexpectedly.
/// * [`GrabError::GitExecutionError`]: If the `git` executable cannot be run.
/// * [`GrabError::GlobMatcherBuildError`]: If exclude patterns are invalid.
///
/// Note that errors reading individual files or encountering non-UTF8 files typically
/// result in logged warnings rather than returning an `Err`, allowing the function
/// to process as many files as possible.
///
/// # Examples
///
/// ```no_run
/// use dirgrab_lib::{GrabConfig, grab_contents, GrabError};
/// use std::path::PathBuf;
///
/// fn run_dirgrab() -> Result<String, GrabError> {
///     let config = GrabConfig {
///         target_path: PathBuf::from("./my_project"), // Target a specific project
///         add_headers: false,                        // Don't include headers
///         exclude_patterns: vec!["target/".to_string()], // Exclude the target dir
///         include_untracked: true,                   // Include untracked files if it's a Git repo
///     };
///
///     grab_contents(&config)
/// }
///
/// match run_dirgrab() {
///     Ok(content) => {
///         println!("Successfully grabbed content ({} bytes).", content.len());
///         // Example: copy to clipboard or send to LLM
///         // use arboard::Clipboard;
///         // if let Ok(mut ctx) = Clipboard::new() {
///         //     ctx.set_text(content).expect("Failed to set clipboard");
///         // }
///     }
///     Err(e) => {
///         eprintln!("Error running dirgrab: {}", e);
///         // Handle the error appropriately
///     }
/// }
/// ```
pub fn grab_contents(config: &GrabConfig) -> GrabResult<String> {
    info!("Starting dirgrab operation with config: {:?}", config);

    // Canonicalize cleans the path and checks existence implicitly via OS call
    let target_path = config.target_path.canonicalize().map_err(|e| {
        // Provide a slightly better error if the root cause is NotFound
        if e.kind() == io::ErrorKind::NotFound {
            GrabError::TargetPathNotFound(config.target_path.clone())
        } else {
            GrabError::IoError {
                path: config.target_path.clone(),
                source: e,
            }
        }
    })?;
    debug!("Canonical target path: {:?}", target_path);

    // 1. Detect Git repository and root
    let git_repo_root = detect_git_repo(&target_path)?;

    // 2. List files based on mode (Git vs. Non-Git)
    let files_to_process = match &git_repo_root {
        Some(root) => {
            info!("Operating in Git mode. Repo root: {:?}", root);
            list_files_git(root, config)?
        }
        None => {
            info!("Operating in Non-Git mode. Target path: {:?}", target_path);
            list_files_walkdir(&target_path, config)?
        }
    };

    info!("Found {} files to process.", files_to_process.len());
    if files_to_process.is_empty() {
        warn!("No files selected for processing based on current configuration.");
        return Ok(String::new()); // Return empty string if no files
    }

    // 3. Process (read and concatenate) the files
    process_files(
        &files_to_process,
        config.add_headers,
        git_repo_root.as_deref(),
    )
}

// --- Helper Function Implementations ---
// (Private functions below - no public API doc comments needed,
// but internal // comments can clarify complex logic if necessary)

/// Checks if the path is inside a Git repository and returns the repo root if true.
fn detect_git_repo(path: &Path) -> GrabResult<Option<PathBuf>> {
    let command_str = "git rev-parse --show-toplevel";
    debug!(
        "Detecting git repo by running '{}' in path: {:?}",
        command_str, path
    );

    // Attempt to run git command, handle specific "not found" error gracefully
    let output = match run_command("git", &["rev-parse", "--show-toplevel"], path) {
        Ok(output) => output,
        Err(GrabError::GitExecutionError { ref source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            // Git command not found, definitely not a Git repo context for us
            info!("'git' command not found. Assuming Non-Git mode.");
            return Ok(None);
        }
        Err(e) => return Err(e), // Propagate other execution errors
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            // Attempt to canonicalize the reported root path for consistency
            let root_path =
                PathBuf::from(stdout)
                    .canonicalize()
                    .map_err(|e| GrabError::IoError {
                        path: PathBuf::from("detected git root"),
                        source: e,
                    })?;
            debug!("Detected Git repo root: {:?}", root_path);
            Ok(Some(root_path))
        } else {
            // Command succeeded but gave empty output? Unexpected. Treat as non-repo.
            warn!(
                "'{}' succeeded but returned empty output in {:?}. Treating as Non-Git mode.",
                command_str, path
            );
            Ok(None)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check stderr for common messages indicating not a git repo
        if stderr.contains("not a git repository")
            || stderr.contains("fatal: detected dubious ownership in repository at")
        {
            debug!(
                "Path is not inside a Git repository (based on stderr): {:?}",
                path
            );
            Ok(None)
        } else {
            // A different git error occurred
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            error!(
                "Git command '{}' failed unexpectedly.\nStderr: {}\\nStdout: {}",
                command_str, stderr, stdout
            );
            Err(GrabError::GitCommandError {
                command: command_str.to_string(),
                stderr: stderr.into_owned(),
                stdout,
            })
        }
    }
}

/// Lists files using `git ls-files`. Handles tracked and optionally untracked files.
fn list_files_git(repo_root: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using Git in root: {:?}", repo_root);

    let base_args = ["ls-files", "-z"]; // Always use null termination for safe parsing
    let exclude_pathspecs: Vec<String> = config
        .exclude_patterns
        .iter()
        .map(|p| format!(":!{}", p)) // Format as git pathspec exclusions
        .collect();
    let exclude_refs: Vec<&str> = exclude_pathspecs.iter().map(AsRef::as_ref).collect();

    let mut combined_files = HashSet::new(); // Use HashSet for automatic deduplication

    // 1. Get TRACKED files (respecting command-line excludes)
    let mut tracked_args = base_args.to_vec();
    tracked_args.extend_from_slice(&exclude_refs);
    let tracked_command_str = format!("git {}", tracked_args.join(" "));
    debug!(
        "Running git command for tracked files: {}",
        tracked_command_str
    );
    let tracked_output = run_command("git", &tracked_args, repo_root)?;
    if !tracked_output.status.success() {
        let stderr = String::from_utf8_lossy(&tracked_output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&tracked_output.stdout).into_owned();
        error!(
            "git ls-files command (tracked) failed.\nStderr: {}\nStdout: {}",
            stderr, stdout
        );
        return Err(GrabError::GitCommandError {
            command: tracked_command_str,
            stderr,
            stdout,
        });
    }
    // Add tracked files found to the set
    String::from_utf8_lossy(&tracked_output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .for_each(|s| {
            combined_files.insert(repo_root.join(s));
        });

    // 2. Get UNTRACKED files (if requested, respecting .gitignore and command-line excludes)
    if config.include_untracked {
        let mut untracked_args = base_args.to_vec();
        untracked_args.push("--others"); // Show untracked files
        untracked_args.push("--exclude-standard"); // IMPORTANT: Still respect .gitignore rules
        untracked_args.extend_from_slice(&exclude_refs); // Apply command-line excludes too
        let untracked_command_str = format!("git {}", untracked_args.join(" "));
        debug!(
            "Running git command for untracked files: {}",
            untracked_command_str
        );
        let untracked_output = run_command("git", &untracked_args, repo_root)?;

        if !untracked_output.status.success() {
            let stderr = String::from_utf8_lossy(&untracked_output.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&untracked_output.stdout).into_owned();
            error!(
                "git ls-files command (untracked) failed.\nStderr: {}\nStdout: {}",
                stderr, stdout
            );
            return Err(GrabError::GitCommandError {
                command: untracked_command_str,
                stderr,
                stdout,
            });
        }
        // Add untracked files found to the set (duplicates are handled by HashSet)
        String::from_utf8_lossy(&untracked_output.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .for_each(|s| {
                combined_files.insert(repo_root.join(s));
            });
    }

    // Convert the combined set back to a Vec for the return type
    let files_vec = combined_files.into_iter().collect();
    Ok(files_vec)
}

/// Lists files using `walkdir` when not in a Git repository. Applies command-line excludes.
fn list_files_walkdir(target_path: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using walkdir starting at: {:?}", target_path);
    let mut files = Vec::new();

    // Build the matcher for command-line exclusion patterns
    let mut exclude_builder = GitignoreBuilder::new(target_path);
    for pattern in &config.exclude_patterns {
        if let Err(e) = exclude_builder.add_line(None, pattern) {
            // Log error for invalid pattern but continue, ignoring the bad pattern
            error!(
                "Failed to add exclude pattern '{}': {}. This pattern will be ignored.",
                pattern, e
            );
        }
    }
    let exclude_matcher = exclude_builder
        .build()
        .map_err(GrabError::GlobMatcherBuildError)?;

    // Walk the directory
    for entry_result in WalkDir::new(target_path) {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
                // Log errors during walk (e.g., permission denied) and skip the entry
                let path_display = e.path().map_or_else(
                    || target_path.display().to_string(),
                    |p| p.display().to_string(),
                );
                warn!(
                    "Skipping path due to error during walk near {}: {}",
                    path_display, e
                );
                continue;
            }
        };

        let path = entry.path();

        // Only process files
        if !entry.file_type().is_file() {
            continue;
        }

        // Apply exclusion rules using the command-line patterns
        match exclude_matcher.matched_path_or_any_parents(path, false) {
            Match::None | Match::Whitelist(_) => {
                // Not ignored by --exclude patterns, add it
                files.push(path.to_path_buf());
            }
            Match::Ignore(_) => {
                // Ignored by an --exclude pattern
                debug!("Excluding file due to pattern (walkdir): {:?} matching pattern for path or parent", path);
                continue; // Skip this file
            }
        }
    } // End walkdir loop

    Ok(files)
}

/// Reads a list of files, concatenates their UTF-8 content, optionally adding headers.
/// Skips non-UTF8 files and files with read errors, logging warnings.
fn process_files(
    files: &[PathBuf],
    add_headers: bool,
    repo_root: Option<&Path>,
) -> GrabResult<String> {
    debug!("Processing {} files.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024); // Preallocate estimate
    let mut buffer = Vec::new(); // Reusable buffer for reading

    for file_path in files {
        debug!("Processing file: {:?}", file_path);

        // --- Add Header if requested ---
        if add_headers {
            // Try to make the path relative to the repo root (if in git mode) or the original target path
            let display_path = repo_root
                .and_then(|root| file_path.strip_prefix(root).ok()) // Attempt strip_prefix if root exists
                .unwrap_or(file_path); // Fallback to the full path if not relative or strip fails

            combined_content.push_str(&format!("--- FILE: {} ---\n", display_path.display()));
        }

        // --- Read File Content ---
        buffer.clear(); // Reuse the buffer
        match fs::File::open(file_path) {
            Ok(file) => {
                let mut reader = BufReader::new(file);
                match reader.read_to_end(&mut buffer) {
                    Ok(_) => {
                        // Attempt to decode as UTF-8
                        match String::from_utf8(buffer.clone()) {
                            // Clone needed as buffer is reused
                            Ok(content) => {
                                combined_content.push_str(&content);
                                // Ensure separation with a newline, even if file doesn't end with one
                                if !content.ends_with('\n') {
                                    combined_content.push('\n');
                                }
                                // Add an extra newline between files for readability
                                combined_content.push('\n');
                            }
                            Err(_) => {
                                // File is not valid UTF-8 (likely binary)
                                warn!("Skipping non-UTF8 file: {:?}", file_path);
                            }
                        }
                    }
                    Err(e) => {
                        // Error reading file content
                        warn!("Skipping file due to read error: {:?} - {}", file_path, e);
                    }
                }
            }
            Err(e) => {
                // Error opening file (e.g., permissions changed since listing)
                warn!("Skipping file due to open error: {:?} - {}", file_path, e);
            }
        }
    }

    Ok(combined_content)
}

/// Utility function to run an external command and capture its output.
fn run_command(cmd: &str, args: &[&str], current_dir: &Path) -> GrabResult<Output> {
    debug!(
        "Running command: {} {:?} in directory: {:?}",
        cmd, args, current_dir
    );
    let output = Command::new(cmd)
        .args(args)
        .current_dir(current_dir) // Execute in the specified directory
        .output()
        // Map I/O errors during execution (like command not found)
        .map_err(|e| {
            let command_string = format!("{} {}", cmd, args.join(" "));
            if e.kind() == std::io::ErrorKind::NotFound {
                // Provide a specific error message if the command wasn't found
                error!(
                    "Command '{}' not found. Is '{}' installed and in your system's PATH?",
                    command_string, cmd
                );
            }
            // Wrap the error in our custom type
            GrabError::GitExecutionError {
                command: command_string,
                source: e,
            }
        })?;

    // Return the captured output (caller checks status code)
    Ok(output)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::collections::HashSet; // For order-independent comparison
    use std::fs::{self};
    use std::path::Path; // Ensure Path is imported directly for test cases
    use std::process::Command;
    use tempfile::{tempdir, TempDir};

    // Helper function to create a basic temporary directory setup
    fn setup_test_dir() -> Result<(TempDir, PathBuf)> {
        let dir = tempdir()?;
        let path = dir.path().to_path_buf();

        fs::write(path.join("file1.txt"), "Content of file 1.")?;
        fs::write(path.join("file2.rs"), "fn main() {}")?;
        fs::create_dir(path.join("subdir"))?;
        fs::write(path.join("subdir").join("file3.log"), "Log message.")?;
        fs::write(
            path.join("subdir").join("another.txt"),
            "Another text file.",
        )?;
        fs::write(path.join("binary.dat"), [0x80, 0x81, 0x82])?;

        Ok((dir, path))
    }

    // Helper function to initialize a git repo in a temp dir
    // Returns Ok(true) if git repo was set up, Ok(false) if git command failed (e.g., not found)
    fn setup_git_repo(path: &Path) -> Result<bool> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("WARN: 'git' command not found, skipping Git-related test setup.");
            return Ok(false); // Indicate git is not available
        }

        run_command_test("git", &["init", "-b", "main"], path)?;
        run_command_test("git", &["config", "user.email", "test@example.com"], path)?;
        run_command_test("git", &["config", "user.name", "Test User"], path)?;

        // Add .gitignore *before* adding files
        // Ignore logs, binary.dat, and specifically file1.txt
        fs::write(path.join(".gitignore"), "*.log\nbinary.dat\nfile1.txt")?;

        run_command_test(
            "git",
            &["add", ".gitignore", "file2.rs", "subdir/another.txt"],
            path,
        )?; // Add specific files + .gitignore
            // Note: file1.txt, binary.dat, subdir/file3.log are NOT added initially

        run_command_test("git", &["commit", "-m", "Initial commit"], path)?;

        // Create an untracked file (that isn't ignored)
        fs::write(path.join("untracked.txt"), "This file is not tracked.")?;
        // Create an explicitly ignored file
        fs::write(path.join("ignored.log"), "This should be ignored by git.")?; // Matches *.log

        Ok(true) // Indicate git setup success
    }

    // Helper to run commands specifically for tests, panicking on failure
    fn run_command_test(cmd: &str, args: &[&str], current_dir: &Path) -> Result<Output> {
        println!(
            "Running test command: {} {:?} in {:?}",
            cmd, args, current_dir
        );
        let output = Command::new(cmd)
            .args(args)
            .current_dir(current_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            anyhow::bail!(
                "Command failed: {} {:?}\nStatus: {}\nStdout: {}\nStderr: {}",
                cmd,
                args,
                output.status,
                stdout,
                stderr
            );
        }
        Ok(output)
    }

    // Helper to convert lists of relative paths to absolute paths in the test repo
    // and then into a HashSet for comparison.
    fn get_expected_set(base_path: &Path, relative_paths: &[&str]) -> HashSet<PathBuf> {
        relative_paths.iter().map(|p| base_path.join(p)).collect()
    }

    fn assert_paths_eq(actual: Vec<PathBuf>, expected: HashSet<PathBuf>) {
        let actual_set: HashSet<PathBuf> = actual.into_iter().collect();
        assert_eq!(actual_set, expected);
    }

    #[test]
    fn test_detect_git_repo_inside() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Skip if git not available

        let maybe_root = detect_git_repo(&path)?;
        assert!(maybe_root.is_some(), "Should detect git repo");
        assert_eq!(maybe_root.unwrap().canonicalize()?, path.canonicalize()?);

        let subdir_path = path.join("subdir");
        let maybe_root_from_subdir = detect_git_repo(&subdir_path)?;
        assert!(
            maybe_root_from_subdir.is_some(),
            "Should detect git repo from subdir"
        );
        assert_eq!(
            maybe_root_from_subdir.unwrap().canonicalize()?,
            path.canonicalize()?
        );

        Ok(())
    }

    #[test]
    fn test_detect_git_repo_outside() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;

        let maybe_root = detect_git_repo(&path)?;
        assert!(maybe_root.is_none(), "Should not detect git repo");
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_no_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
        };

        let files = list_files_walkdir(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                "file1.txt",
                "file2.rs",
                "subdir/file3.log",
                "subdir/another.txt",
                "binary.dat",
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["*.log".to_string(), "subdir/".to_string()],
            include_untracked: false,
        };

        let files = list_files_walkdir(&path, &config)?;

        let expected_set = get_expected_set(&path, &["file1.txt", "file2.rs", "binary.dat"]);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    // --- NEW Git Tests ---

    #[test]
    fn test_list_files_git_tracked_only() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(), // Not directly used by list_files_git, but needed
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false, // Default: only tracked files
        };

        let files = list_files_git(&path, &config)?;

        // Expected: Only files explicitly added and committed (.gitignore, file2.rs, subdir/another.txt)
        let expected_set =
            get_expected_set(&path, &[".gitignore", "file2.rs", "subdir/another.txt"]);

        println!("Git tracked files found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_untracked() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true, // The key flag for this test
        };

        let files = list_files_git(&path, &config)?;

        // Expected: tracked files + untracked.txt
        // .gitignore'd files (file1.txt, binary.dat, *.log) should NOT be included
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "untracked.txt", // The untracked file
            ],
        );

        println!("Git tracked+untracked files found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            // Exclude Rust files and everything in subdir/
            exclude_patterns: vec!["*.rs".to_string(), "subdir/".to_string()],
            include_untracked: false, // Tracked only
        };

        let files = list_files_git(&path, &config)?;

        // Expected: .gitignore (file2.rs and subdir/another.txt are excluded)
        let expected_set = get_expected_set(&path, &[".gitignore"]);

        println!("Git tracked files (with exclude) found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_untracked_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            // Exclude .txt files
            exclude_patterns: vec!["*.txt".to_string()],
            include_untracked: true, // Include untracked
        };

        let files = list_files_git(&path, &config)?;

        // Expected: .gitignore, file2.rs
        // Excluded: subdir/another.txt, untracked.txt
        let expected_set = get_expected_set(&path, &[".gitignore", "file2.rs"]);

        println!(
            "Git tracked+untracked (with exclude) files found: {:?}",
            files
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    // --- End of NEW Git Tests ---

    #[test]
    fn test_process_files_no_headers_skip_binary() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![
            path.join("file1.txt"),
            path.join("binary.dat"),
            path.join("file2.rs"),
        ];

        let result = process_files(&files_to_process, false, None)?;

        let expected_content = "Content of file 1.\n\nfn main() {}\n\n";

        assert_eq!(result.trim(), expected_content.trim());

        Ok(())
    }

    #[test]
    fn test_process_files_with_headers() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("file2.rs")];

        let repo_root = Some(path.as_path());

        let result = process_files(&files_to_process, true, repo_root)?;

        let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nfn main() {{}}\n\n",
            Path::new("file1.txt").display(), // Use Path::new for consistent display across OS
            Path::new("file2.rs").display()
        );

        assert_eq!(result.trim(), expected_content.trim());

        Ok(())
    }
} // End of mod tests
