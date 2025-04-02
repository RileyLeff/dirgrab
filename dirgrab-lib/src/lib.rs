// --- FILE: dirgrab-lib/src/lib.rs ---

#![doc = include_str!("../README.md")]

use ignore::gitignore::GitignoreBuilder;
use ignore::Match;
use std::collections::HashSet;
use std::fs; // Used in process_files
use std::io::{self, BufReader, Read}; // Used in process_files
use std::path::{Path, PathBuf};
use std::process::{Command, Output}; // Used in run_command
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
    /// This setting has no effect if the `target_path` is not part of a Git repository,
    /// or if `no_git` is true.
    pub include_untracked: bool,

    /// If true, the default exclusion for `dirgrab.txt` will *not* be applied.
    /// Use this flag only if you specifically need to include a file named `dirgrab.txt`.
    pub include_default_output: bool,

    /// If true, forces dirgrab to ignore any Git repository context and treat the
    /// target path purely as a filesystem directory. This disables `.gitignore`
    /// processing and the effect of `include_untracked`. User-provided exclude
    /// patterns (`-e`) are still respected.
    pub no_git: bool,
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
///         add_headers: true,                         // Include headers
///         exclude_patterns: vec!["target/".to_string()], // Exclude the target dir
///         include_untracked: false,                  // Only tracked files (if Git)
///         include_default_output: false,             // Exclude dirgrab.txt
///         no_git: false,                             // Use Git context if available
///         // include_tree: false, // Example for Item D
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

    // Determine file listing mode and potential repo root based on no_git flag
    let (files_to_process, maybe_repo_root) = if config.no_git {
        // Force non-Git mode (walkdir)
        info!("Ignoring Git context due to --no-git flag.");
        let files = list_files_walkdir(&target_path, config)?;
        // When forcing non-git, there's no repo root concept for headers
        (files, None)
    } else {
        // Default behavior: Detect Git repo and choose mode
        let git_repo_root = detect_git_repo(&target_path)?;
        let files = match &git_repo_root {
            Some(root) => {
                info!("Operating in Git mode. Repo root: {:?}", root);
                list_files_git(root, config)?
            }
            None => {
                info!("Operating in Non-Git mode. Target path: {:?}", target_path);
                list_files_walkdir(&target_path, config)?
            }
        };
        // Pass the detected repo root (if any) for header relativization
        (files, git_repo_root)
    };

    info!("Found {} files to process.", files_to_process.len());
    if files_to_process.is_empty() {
        warn!("No files selected for processing based on current configuration.");
        return Ok(String::new()); // Return empty string if no files
    }

    // Process (read and concatenate) the files, passing the canonical target path for non-git relativization
    process_files(
        &files_to_process,
        config.add_headers,
        maybe_repo_root.as_deref(), // Use the root determined above (if any)
        &target_path,               // Pass the canonical target path for fallback relativization
    )
}

// --- Helper Function Implementations ---

/// Checks if the path is inside a Git repository and returns the repo root if true.
fn detect_git_repo(path: &Path) -> GrabResult<Option<PathBuf>> {
    let command_str = "git rev-parse --show-toplevel";
    debug!(
        "Detecting git repo by running '{}' in path: {:?}",
        command_str, path
    );

    let output = match run_command("git", &["rev-parse", "--show-toplevel"], path) {
        Ok(output) => output,
        Err(GrabError::GitExecutionError { ref source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            info!("'git' command not found. Assuming Non-Git mode.");
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
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
            warn!(
                "'{}' succeeded but returned empty output in {:?}. Treating as Non-Git mode.",
                command_str, path
            );
            Ok(None)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("not a git repository")
            || stderr.contains("fatal: detected dubious ownership in repository at")
        {
            debug!(
                "Path is not inside a Git repository (based on stderr): {:?}",
                path
            );
            Ok(None)
        } else {
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

    let base_args = ["ls-files", "-z"];
    let exclude_pathspecs: Vec<String> = config
        .exclude_patterns
        .iter()
        .map(|p| format!(":!{}", p))
        .collect();

    let mut all_exclude_refs: Vec<&str> = Vec::new();
    if !config.include_default_output {
        let default_exclude_pathspec = ":!dirgrab.txt";
        all_exclude_refs.push(default_exclude_pathspec);
        debug!("Applying default exclusion for 'dirgrab.txt'");
    } else {
        info!("Default exclusion for 'dirgrab.txt' is disabled by --include-default-output flag.");
    }
    all_exclude_refs.extend(exclude_pathspecs.iter().map(|s| s.as_str()));

    let mut combined_files = HashSet::new();

    // Tracked files
    let mut tracked_args = base_args.to_vec();
    tracked_args.extend_from_slice(&all_exclude_refs);
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
    String::from_utf8_lossy(&tracked_output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .for_each(|s| {
            combined_files.insert(repo_root.join(s));
        });

    // Untracked files (if requested)
    if config.include_untracked {
        let mut untracked_args = base_args.to_vec();
        untracked_args.push("--others");
        untracked_args.push("--exclude-standard");
        untracked_args.extend_from_slice(&all_exclude_refs);
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
        String::from_utf8_lossy(&untracked_output.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .for_each(|s| {
                combined_files.insert(repo_root.join(s));
            });
    }

    Ok(combined_files.into_iter().collect())
}

/// Lists files using `walkdir` when not in a Git repository. Applies command-line excludes.
fn list_files_walkdir(target_path: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using walkdir starting at: {:?}", target_path);
    let mut files = Vec::new();

    let mut exclude_builder = GitignoreBuilder::new(target_path);

    if !config.include_default_output {
        if let Err(e) = exclude_builder.add_line(None, "dirgrab.txt") {
            warn!("Failed to add default exclusion pattern 'dirgrab.txt': {}. This exclusion might not apply.", e);
        } else {
            debug!("Applying default exclusion for 'dirgrab.txt'");
        }
    } else {
        info!("Default exclusion for 'dirgrab.txt' is disabled by --include-default-output flag.");
    }

    for pattern in &config.exclude_patterns {
        if let Err(e) = exclude_builder.add_line(None, pattern) {
            error!(
                "Failed to add exclude pattern '{}': {}. This pattern will be ignored.",
                pattern, e
            );
        }
    }
    let exclude_matcher = exclude_builder
        .build()
        .map_err(GrabError::GlobMatcherBuildError)?;

    for entry_result in WalkDir::new(target_path) {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
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

        // Skip directories and other non-files
        if !entry.file_type().is_file() {
            // Also explicitly skip the .git directory if walkdir happens upon it
            // (though it shouldn't typically list its contents unless target_path *is* .git)
            if path.file_name() == Some(std::ffi::OsStr::new(".git")) && entry.file_type().is_dir()
            {
                debug!("Skipping .git directory during walkdir");
                continue;
            }
            continue;
        }

        // Apply exclusion rules using the patterns
        match exclude_matcher.matched_path_or_any_parents(path, false) {
            Match::None | Match::Whitelist(_) => {
                files.push(path.to_path_buf());
            }
            Match::Ignore(_) => {
                debug!("Excluding file due to pattern (walkdir): {:?} matching pattern for path or parent", path);
                continue;
            }
        }
    }

    Ok(files)
}

/// Reads a list of files, concatenates their UTF-8 content, optionally adding headers.
/// Skips non-UTF8 files and files with read errors, logging warnings.
fn process_files(
    files: &[PathBuf],
    add_headers: bool,
    repo_root: Option<&Path>,
    target_path: &Path, // Added parameter for non-git relative paths
) -> GrabResult<String> {
    debug!("Processing {} files.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024);
    let mut buffer = Vec::new();

    for file_path in files {
        debug!("Processing file: {:?}", file_path);

        if add_headers {
            // Decide which path to strip based on whether repo_root is Some
            let display_path_result = match repo_root {
                // Git mode: try stripping repo root
                Some(root) => file_path.strip_prefix(root),
                // Non-Git mode OR --no-git: try stripping target path
                None => file_path.strip_prefix(target_path),
            };

            // Use the relativized path if successful, otherwise fall back to the absolute path
            let display_path = display_path_result.unwrap_or(file_path);

            combined_content.push_str(&format!("--- FILE: {} ---\n", display_path.display()));
        }

        buffer.clear();
        match fs::File::open(file_path) {
            Ok(file) => {
                let mut reader = BufReader::new(file);
                match reader.read_to_end(&mut buffer) {
                    Ok(_) => {
                        match String::from_utf8(buffer.clone()) {
                            Ok(content) => {
                                combined_content.push_str(&content);
                                if !content.ends_with('\n') {
                                    combined_content.push('\n');
                                }
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
        .current_dir(current_dir)
        .output()
        .map_err(|e| {
            let command_string = format!("{} {}", cmd, args.join(" "));
            if e.kind() == std::io::ErrorKind::NotFound {
                error!(
                    "Command '{}' not found. Is '{}' installed and in your system's PATH?",
                    command_string, cmd
                );
            }
            GrabError::GitExecutionError {
                command: command_string,
                source: e,
            }
        })?;

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
        // Add the default output file to test exclusion
        fs::write(path.join("dirgrab.txt"), "Previous dirgrab output.")?;

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
        // NOTE: We DO NOT ignore dirgrab.txt here; its exclusion should come from the tool itself.
        fs::write(path.join(".gitignore"), "*.log\nbinary.dat\nfile1.txt")?;

        run_command_test(
            "git",
            &["add", ".gitignore", "file2.rs", "subdir/another.txt"],
            path,
        )?; // Add specific files + .gitignore
            // Note: file1.txt, binary.dat, subdir/file3.log, dirgrab.txt are NOT added initially

        run_command_test("git", &["commit", "-m", "Initial commit"], path)?;

        // Create an untracked file (that isn't ignored)
        fs::write(path.join("untracked.txt"), "This file is not tracked.")?;
        // Create an explicitly ignored file
        fs::write(path.join("ignored.log"), "This should be ignored by git.")?; // Matches *.log

        // Create nested structure for header test
        fs::create_dir_all(path.join("deep/sub"))?;
        fs::write(path.join("deep/sub/nested.txt"), "Nested content")?;
        run_command_test("git", &["add", "deep/sub/nested.txt"], path)?; // Track the nested file
        run_command_test("git", &["commit", "-m", "Add nested file"], path)?;

        Ok(true) // Indicate git setup success
    }

    // Helper to run commands specifically for tests, panicking on failure
    fn run_command_test(cmd: &str, args: &[&str], current_dir: &Path) -> Result<Output> {
        // println!( // Keep commented out unless debugging test commands
        //     "Running test command: {} {:?} in {:?}",
        //     cmd, args, current_dir
        // );
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

    // --- Existing Tests (Adapted with no_git: false) ---

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
    fn test_list_files_walkdir_no_exclude_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: false, // Explicitly default behavior
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
            include_default_output: false,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_walkdir(&path, &config)?;

        let expected_set = get_expected_set(&path, &["file1.txt", "file2.rs", "binary.dat"]);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_tracked_only_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
            ],
        );

        //println!("Git tracked files found: {:?}", files); // Keep commented out unless debugging
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_untracked_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,
            include_default_output: false,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt",
            ],
        );

        //println!("Git tracked+untracked files found: {:?}", files); // Keep commented out unless debugging
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![
                "*.rs".to_string(),
                "subdir/".to_string(),
                "deep/".to_string(),
            ], // Exclude nested too
            include_untracked: false,
            include_default_output: false,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(&path, &[".gitignore"]);

        //println!("Git tracked files (with exclude) found: {:?}", files); // Keep commented out unless debugging
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_untracked_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["*.txt".to_string()], // Excludes untracked.txt, another.txt, nested.txt
            include_untracked: true,
            include_default_output: false,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(&path, &[".gitignore", "file2.rs"]);

        // println!( // Keep commented out unless debugging
        //     "Git tracked+untracked (with exclude) files found: {:?}",
        //     files
        // );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    // --- Tests for Item (B) - Specific Override (Adapted with no_git: false) ---

    #[test]
    fn test_list_files_walkdir_include_default_output() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true,
            no_git: false, // Explicitly default behavior
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
                "dirgrab.txt",
                // Note: deep/sub/nested.txt is not created in non-git setup
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_tracked_only() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        fs::write(path.join("dirgrab.txt"), "Tracked dirgrab output.")?;
        run_command_test("git", &["add", "dirgrab.txt"], &path)?;
        run_command_test("git", &["commit", "-m", "Add dirgrab.txt"], &path)?;

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "dirgrab.txt",
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_with_untracked() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }
        // dirgrab.txt is untracked here (created by setup_test_dir)

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,
            include_default_output: true,
            no_git: false, // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt",
                "dirgrab.txt",
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_but_excluded_by_user() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["dirgrab.txt".to_string()],
            include_untracked: true,
            include_default_output: true, // Override doesn't matter
            no_git: false,                // Explicitly default behavior
        };

        let files = list_files_git(&path, &config)?;

        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt",
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    // --- Tests for Item (C) - Specific --no-git Behavior ---

    #[test]
    fn test_no_git_flag_forces_walkdir_in_git_repo() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,      // Should have no effect
            include_default_output: false, // Exclude dirgrab.txt
            no_git: true,                  // Force walkdir
        };

        // Use grab_contents to test the top-level logic switch
        let result_string = grab_contents(&config)?;

        // Check content presence/absence based on walkdir behavior
        assert!(
            result_string.contains("Content of file 1."),
            "Should include file1.txt (ignored by .gitignore but --no-git)"
        );
        assert!(
            result_string.contains("*.log"),
            "Should include .gitignore itself"
        );
        assert!(
            result_string.contains("fn main() {}"),
            "Should include file2.rs"
        );
        assert!(
            result_string.contains("Log message."),
            "Should include file3.log (ignored by .gitignore)"
        );
        assert!(
            result_string.contains("Another text file."),
            "Should include another.txt"
        );
        assert!(
            result_string.contains("Nested content"),
            "Should include nested.txt"
        );
        assert!(
            result_string.contains("This file is not tracked."),
            "Should include untracked.txt"
        );
        assert!(
            result_string.contains("This should be ignored by git."),
            "Should include ignored.log"
        );
        assert!(
            !result_string.contains("Previous dirgrab output."),
            "Should exclude dirgrab.txt by default"
        );

        Ok(())
    }

    #[test]
    fn test_no_git_flag_still_respects_exclude_patterns() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["*.txt".to_string(), "*.rs".to_string()], // Exclude txt and rs
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Force walkdir
        };

        let result_string = grab_contents(&config)?;

        assert!(result_string.contains("*.log"), "Should include .gitignore");
        assert!(
            result_string.contains("Log message."),
            "Should include file3.log"
        );
        assert!(
            result_string.contains("This should be ignored by git."),
            "Should include ignored.log"
        );

        assert!(
            !result_string.contains("Content of file 1."),
            "Should exclude file1.txt by -e"
        );
        assert!(
            !result_string.contains("fn main() {}"),
            "Should exclude file2.rs by -e"
        );
        assert!(
            !result_string.contains("Another text file."),
            "Should exclude another.txt by -e"
        );
        assert!(
            !result_string.contains("Nested content"),
            "Should exclude nested.txt by -e"
        );
        assert!(
            !result_string.contains("This file is not tracked."),
            "Should exclude untracked.txt by -e"
        );
        assert!(
            !result_string.contains("Previous dirgrab output."),
            "Should exclude dirgrab.txt by default"
        );

        Ok(())
    }

    #[test]
    fn test_no_git_flag_with_include_default_output() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true, // Include dirgrab.txt
            no_git: true,                 // Force walkdir
        };

        let result_string = grab_contents(&config)?;

        assert!(result_string.contains("Content of file 1."));
        assert!(result_string.contains("*.log"));
        assert!(result_string.contains("fn main() {}"));
        assert!(result_string.contains("Log message."));
        assert!(result_string.contains("Another text file."));
        assert!(result_string.contains("Nested content"));
        assert!(result_string.contains("This file is not tracked."));
        assert!(result_string.contains("This should be ignored by git."));
        assert!(
            result_string.contains("Previous dirgrab output."),
            "Should include dirgrab.txt due to override"
        );

        Ok(())
    }

    #[test]
    fn test_no_git_flag_headers_relative_to_target() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        // nested.txt is created and tracked by setup_git_repo

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true, // Enable headers
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Force walkdir
        };

        let result_string = grab_contents(&config)?;

        // When --no-git is used, repo_root is None, so paths should be relative to target_path
        // Use Path::new for cross-platform path separator consistency in expected header
        let expected_nested_header = format!(
            "--- FILE: {} ---",
            Path::new("deep/sub/nested.txt").display()
        );
        assert!(
            result_string.contains(&expected_nested_header),
            "Header path should be relative to target_path. Expected '{}' in output:\n{}",
            expected_nested_header,
            result_string
        );

        let expected_root_header = format!("--- FILE: {} ---", Path::new(".gitignore").display());
        assert!(
            result_string.contains(&expected_root_header),
            "Root file header check. Expected '{}' in output:\n{}",
            expected_root_header,
            result_string
        );

        Ok(())
    }

    #[test]
    fn test_git_mode_headers_relative_to_repo_root() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }

        // Use a target path *inside* the repo
        let subdir_target = path.join("deep");

        let config = GrabConfig {
            target_path: subdir_target.clone(), // Start search from subdir
            add_headers: true,                  // Enable headers
            exclude_patterns: vec![],
            include_untracked: false, // Tracked only
            include_default_output: false,
            no_git: false, // Use Git mode
        };

        let result_string = grab_contents(&config)?;

        // Even though target_path is subdir, headers should be relative to repo root (path)
        let expected_nested_header = format!(
            "--- FILE: {} ---",
            Path::new("deep/sub/nested.txt").display()
        );
        assert!(
            result_string.contains(&expected_nested_header),
            "Header path should be relative to repo root. Expected '{}' in output:\n{}",
            expected_nested_header,
            result_string
        );

        // Check that files outside the subdir_target but inside repo root are also included and relative
        let expected_root_header = format!("--- FILE: {} ---", Path::new(".gitignore").display());
        assert!(
            result_string.contains(&expected_root_header),
            "Root file header check. Expected '{}' in output:\n{}",
            expected_root_header,
            result_string
        );
        let expected_rs_header = format!("--- FILE: {} ---", Path::new("file2.rs").display());
        assert!(
            result_string.contains(&expected_rs_header),
            "Root rs file header check. Expected '{}' in output:\n{}",
            expected_rs_header,
            result_string
        );

        Ok(())
    }

    // --- Tests for process_files (Adapted with target_path) ---

    #[test]
    fn test_process_files_no_headers_skip_binary() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![
            path.join("file1.txt"),
            path.join("binary.dat"),
            path.join("file2.rs"),
        ];
        let dummy_target_path = path.clone();
        let result = process_files(&files_to_process, false, None, &dummy_target_path)?;

        let expected_content = "Content of file 1.\n\nfn main() {}\n\n";
        assert_eq!(result.trim(), expected_content.trim());
        Ok(())
    }

    #[test]
    fn test_process_files_with_headers_git_mode() -> Result<()> {
        // Test when repo_root is Some (Git mode)
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("file2.rs")];
        let repo_root = Some(path.as_path());
        let dummy_target_path = path.clone(); // Target path still needed but repo_root takes precedence for headers
        let result = process_files(&files_to_process, true, repo_root, &dummy_target_path)?;

        let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nfn main() {{}}\n\n",
            Path::new("file1.txt").display(), // Relative to repo_root
            Path::new("file2.rs").display()   // Relative to repo_root
        );
        assert_eq!(result.trim(), expected_content.trim());
        Ok(())
    }

    #[test]
    fn test_process_files_headers_no_git_mode() -> Result<()> {
        // Test when repo_root is None (Non-Git or --no-git mode)
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("subdir/another.txt")];
        let target_path_ref = path.as_path();

        let result = process_files(&files_to_process, true, None, target_path_ref)?;

        let expected_header1 = format!("--- FILE: {} ---", Path::new("file1.txt").display()); // Relative to target_path
        let expected_header2 = format!(
            "--- FILE: {} ---",
            Path::new("subdir/another.txt").display()
        ); // Relative to target_path

        assert!(
            result.contains(&expected_header1),
            "Header for root file incorrect. Found:\n{}",
            result
        );
        assert!(
            result.contains(&expected_header2),
            "Header for subdir file incorrect. Found:\n{}",
            result
        );
        assert!(result.contains("Content of file 1."));
        assert!(result.contains("Another text file."));

        Ok(())
    }
} // End of mod tests
