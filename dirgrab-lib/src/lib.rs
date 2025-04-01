use ignore::gitignore::GitignoreBuilder; // Keep GitignoreBuilder
use ignore::Match;
use std::fs; // Keep for process_files later
use std::io::{self, BufReader, Read}; // Keep for process_files later
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use thiserror::Error;
use walkdir::WalkDir; // Keep Match

// Re-export log macros for convenience
use log::{debug, error, info, warn};

// --- Public Configuration Struct ---

/// Configuration for the dirgrab operation.
#[derive(Debug, Clone)]
pub struct GrabConfig {
    /// The starting path for the operation.
    pub target_path: PathBuf,
    /// Add '--- FILE: <filename> ---' headers.
    pub add_headers: bool,
    /// Glob patterns to exclude files/directories.
    pub exclude_patterns: Vec<String>,
    /// [Git Mode Only] Include untracked files.
    pub include_untracked: bool,
    // Add other relevant options derived from CLI here if needed
}

// --- Public Error Enum ---

/// Errors that can occur during the dirgrab operation.
#[derive(Error, Debug)]
pub enum GrabError {
    #[error("Target path not found or not accessible: {0}")]
    TargetPathNotFound(PathBuf),
    #[error("IO error accessing path '{path}': {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Failed to execute git command: {command:?}\n  stderr: {stderr}\n  stdout: {stdout}")]
    GitCommandError {
        command: String,
        stderr: String,
        stdout: String,
    },
    #[error("Failed to run git command '{command}': {source}")]
    GitExecutionError {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("Failed to read non-UTF8 file: {0}")]
    NonUtf8File(PathBuf), // Will be used in process_files
    #[error("Could not determine repository root for: {0}")]
    RepoRootNotFound(PathBuf), // Might not be needed if detect_git_repo handles failure gracefully
    #[error("Failed to build glob pattern matcher: {0}")]
    GlobMatcherBuildError(#[source] ignore::Error),
    #[error("Error walking directory {path_display}: {source_str}")]
    WalkdirError {
        path_display: String, // Store a displayable path string
        source_str: String,   // Store the error message
    },
}

// Alias for Result type using our custom error
pub type GrabResult<T> = Result<T, GrabError>;

// --- Main Public Function ---

/// Performs the dirgrab operation based on the provided configuration.
///
/// Reads files from the target directory, potentially respecting Git context,
/// applies exclusions, and concatenates their content into a single string.
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

    // Note: canonicalize already checks existence, so this isn't strictly needed anymore.
    // if !target_path.exists() {
    //     return Err(GrabError::TargetPathNotFound(config.target_path.clone()));
    // }

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

/// Checks if the path is inside a Git repository and returns the repo root if true.
fn detect_git_repo(path: &Path) -> GrabResult<Option<PathBuf>> {
    let command_str = "git rev-parse --show-toplevel";
    debug!(
        "Detecting git repo by running '{}' in path: {:?}",
        command_str, path
    );

    let output = run_command("git", &["rev-parse", "--show-toplevel"], path)?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() {
            // Canonicalize the repo root path for consistency
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
                "'{}' succeeded but returned empty output in {:?}",
                command_str, path
            );
            Ok(None)
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check for specific error indicating "not a git repository"
        // Use `contains` for broader compatibility with git versions/outputs
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
                "Git command '{}' failed unexpectedly.\nStderr: {}\nStdout: {}",
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

/// Lists files using `git ls-files`.
fn list_files_git(repo_root: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    // TODO: Implement using `std::process::Command` to run `git ls-files`
    //       - Pass repo_root as current_dir()
    //       - Add `--others --exclude-standard` if config.include_untracked
    //       - Add `':!<pattern>'` for each config.exclude_patterns
    //       - Parse output lines into PathBufs relative to repo_root
    debug!("Listing files using Git in root: {:?}", repo_root);
    let mut args = vec!["ls-files", "-z"]; // Use -z for null termination (safer parsing)

    if config.include_untracked {
        args.push("--others");
        args.push("--exclude-standard"); // Important to respect .gitignore even for untracked
    }

    // Add exclusion pathspecs (must come after other flags potentially)
    // Prepend the pathspec prefix ':!'
    let exclude_pathspecs: Vec<String> = config
        .exclude_patterns
        .iter()
        .map(|p| format!(":!{}", p))
        .collect();
    // Need to convert Vec<String> to Vec<&str> for args
    let exclude_refs: Vec<&str> = exclude_pathspecs.iter().map(AsRef::as_ref).collect();
    args.extend_from_slice(&exclude_refs);

    let command_str = format!("git {}", args.join(" "));
    debug!("Running git ls-files command: {}", command_str);

    let output = run_command("git", &args, repo_root)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        error!(
            "git ls-files command failed.\nStderr: {}\nStdout: {}",
            stderr, stdout
        );
        return Err(GrabError::GitCommandError {
            command: command_str,
            stderr,
            stdout,
        });
    }

    // Parse the null-terminated output
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let files = stdout_str
        .split('\0') // Split by null character
        .filter(|s| !s.is_empty()) // Filter out potential empty strings
        .map(|s| repo_root.join(s)) // Create absolute paths relative to repo root
        .collect();

    Ok(files)
}

/// Lists files using `walkdir`.
fn list_files_walkdir(target_path: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using walkdir starting at: {:?}", target_path);
    let mut files = Vec::new();

    // --- Build Exclusion Matcher ---
    // We build it relative to the target_path where the walk starts.
    let mut exclude_builder = GitignoreBuilder::new(target_path);
    for pattern in &config.exclude_patterns {
        // The `ignore` crate handles various glob syntaxes.
        // Adding with `None` assumes the pattern applies from the root (target_path).
        if let Err(e) = exclude_builder.add_line(None, pattern) {
            // Log the error but try to continue. A bad pattern shouldn't stop the whole process.
            error!(
                "Failed to add exclude pattern '{}': {}. This pattern will be ignored.",
                pattern, e
            );
            // Optionally return error: return Err(GrabError::GlobMatcherBuildError(e));
        }
    }
    // Build the matcher. This can fail if patterns are fundamentally invalid.
    let exclude_matcher = exclude_builder
        .build()
        .map_err(GrabError::GlobMatcherBuildError)?;
    // --- ---

    // --- Walk Directory ---
    // `WalkDir::new` follows symlinks by default. Consider adding `.follow_links(false)` if needed.
    for entry_result in WalkDir::new(target_path) {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
                // An error during directory traversal (e.g., permissions)
                let path_display = e.path().map_or_else(
                    || target_path.display().to_string(),
                    |p| p.display().to_string(),
                );
                // Log the error and skip this entry/subtree.
                // Returning an error here might be too strict if only one subdir is inaccessible.
                warn!(
                    "Skipping path due to error during walk near {}: {}",
                    path_display, e
                );
                // Optionally return error:
                // return Err(GrabError::WalkdirError {
                //     path_display,
                //     source_str: e.to_string(),
                // });
                continue; // Skip this entry
            }
        };

        let path = entry.path();

        // --- Filter non-files ---
        if !entry.file_type().is_file() {
            continue;
        }

        // --- Apply exclusion rules ---
        // Use `matched_path_or_any_parents` to correctly handle directory exclusions (e.g., `target/`).
        // `is_dir` is `false` here since we already filtered for files.
        match exclude_matcher.matched(path, false) {
            // Use `matched` for files
            Match::None | Match::Whitelist(_) => {
                // Not ignored by --exclude patterns, add it
                files.push(path.to_path_buf());
                // debug!("Including file (walkdir): {:?}", path); // Can be very verbose
            }
            Match::Ignore(_) => {
                // Ignored by --exclude pattern
                debug!("Excluding file due to pattern (walkdir): {:?}", path);
                continue; // Skip this file
            }
        }
    } // End walkdir loop

    Ok(files)
}

/// Reads a list of files and concatenates their content.
fn process_files(
    files: &[PathBuf],
    add_headers: bool,
    repo_root: Option<&Path>,
) -> GrabResult<String> {
    debug!("Processing {} files.", files.len());
    // Estimate capacity: average file size * num files? Hard to guess. Start reasonably.
    let mut combined_content = String::with_capacity(files.len() * 1024); // Guess 1KB avg
    let mut buffer = Vec::new(); // Reusable buffer for reading file contents

    for file_path in files {
        debug!("Processing file: {:?}", file_path);

        // --- Add Header ---
        if add_headers {
            // Try to make the path relative to the repo root (if in git mode) or the original target path
            let display_path = repo_root
                .and_then(|root| file_path.strip_prefix(root).ok())
                .unwrap_or(file_path); // Fallback to absolute path if not in root or strip fails

            combined_content.push_str(&format!("--- FILE: {} ---\n", display_path.display()));
        }

        // --- Read File Content ---
        buffer.clear(); // Clear buffer for next file
        match fs::File::open(file_path) {
            Ok(file) => {
                let mut reader = BufReader::new(file);
                match reader.read_to_end(&mut buffer) {
                    Ok(_) => {
                        // Try to decode as UTF-8
                        match String::from_utf8(buffer.clone()) {
                            // Clone buffer data for String conversion
                            Ok(content) => {
                                combined_content.push_str(&content);
                                // Add a newline if the file doesn't end with one, for separation
                                if !content.ends_with('\n') {
                                    combined_content.push('\n');
                                }
                                combined_content.push('\n'); // Add extra newline between files
                            }
                            Err(_) => {
                                warn!("Skipping non-UTF8 file: {:?}", file_path);
                                // Record the error information if needed later
                                // Optionally return GrabError::NonUtf8File(file_path.clone());
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Skipping file due to read error: {:?} - {}", file_path, e);
                        // Optionally return error:
                        // return Err(GrabError::IoError { path: file_path.clone(), source: e });
                    }
                }
            }
            Err(e) => {
                // Handle file open errors (permissions, not found after listing, etc.)
                warn!("Skipping file due to open error: {:?} - {}", file_path, e);
                // Optionally return error:
                // return Err(GrabError::IoError { path: file_path.clone(), source: e });
            }
        } // End match fs::File::open
    } // End loop over files

    Ok(combined_content)
}

/// Utility function to run a command and capture output
fn run_command(cmd: &str, args: &[&str], current_dir: &Path) -> GrabResult<Output> {
    debug!(
        "Running command: {} {:?} in directory: {:?}",
        cmd, args, current_dir
    );
    let output = Command::new(cmd)
        .args(args)
        .current_dir(current_dir) // Run commands relative to this directory
        .output()
        .map_err(|e| {
            // Improve error message specifically for "command not found"
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
            } // Wrap other execution errors
        })?;

    // Caller is responsible for checking output.status and handling stdout/stderr
    Ok(output)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*; // Import items from the parent module (the library)
    use std::fs::{self, File};
    use std::io::Write;
    use std::process::Command;
    use tempfile::{tempdir, TempDir}; // Use tempfile for temporary test directories

    // Helper function to create a basic temporary directory setup
    fn setup_test_dir() -> anyhow::Result<(TempDir, PathBuf)> {
        let dir = tempdir()?;
        let path = dir.path().to_path_buf();

        // Create some files and directories
        fs::write(path.join("file1.txt"), "Content of file 1.")?;
        fs::write(path.join("file2.rs"), "fn main() {}")?;
        fs::create_dir(path.join("subdir"))?;
        fs::write(path.join("subdir").join("file3.log"), "Log message.")?;
        fs::write(path.join("subdir").join("another.txt"), "Another text file.")?;
        // Create a dummy binary file (non-utf8 sequence)
        fs::write(path.join("binary.dat"), &[0x80, 0x81, 0x82])?;

        Ok((dir, path))
    }

    // Helper function to initialize a git repo in a temp dir
    fn setup_git_repo(path: &Path) -> anyhow::Result<()> {
        // Check if git exists first to avoid test failure if not installed
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("WARN: 'git' command not found, skipping Git-related test setup.");
            // Or panic!("Git command not found, cannot run Git tests.");
            return Ok(()); // Or return an error specific to git missing?
        }

        // Basic git init and commit
        run_command_test("git", &["init", "-b", "main"], path)?; // Init with default branch main
        run_command_test("git", &["config", "user.email", "test@example.com"], path)?; // Need config for commits
        run_command_test("git", &["config", "user.name", "Test User"], path)?;
        run_command_test("git", &["add", "."], path)?; // Add all created files
        run_command_test("git", &["commit", "-m", "Initial commit"], path)?;

        // Create an untracked file
        fs::write(path.join("untracked.txt"), "This file is not tracked.")?;

        // Create an ignored file via .gitignore
        fs::write(path.join(".gitignore"), "*.log\nbinary.dat")?;
        fs::write(path.join("ignored.log"), "This should be ignored by git.")?; // Matches *.log

        Ok(())
    }

     // Helper to run commands specifically for tests, panicking on failure
     fn run_command_test(cmd: &str, args: &[&str], current_dir: &Path) -> anyhow::Result<Output> {
        println!("Running test command: {} {:?} in {:?}", cmd, args, current_dir);
        let output = Command::new(cmd)
            .args(args)
            .current_dir(current_dir)
            .output()?; // Use anyhow's context via ?

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
             anyhow::bail!( // Use anyhow::bail! to fail the test with a message
                 "Command failed: {} {:?}\nStatus: {}\nStdout: {}\nStderr: {}",
                 cmd, args, output.status, stdout, stderr
             );
        }
        Ok(output)
    }


    #[test]
    fn test_detect_git_repo_inside() -> anyhow::Result<()> {
        let (_dir, path) = setup_test_dir()?;
        setup_git_repo(&path)?; // Make it a git repo

        let maybe_root = detect_git_repo(&path)?;
        assert!(maybe_root.is_some(), "Should detect git repo");
        assert_eq!(maybe_root.unwrap().canonicalize()?, path.canonicalize()?);

        // Test from subdirectory
        let subdir_path = path.join("subdir");
         let maybe_root_from_subdir = detect_git_repo(&subdir_path)?;
         assert!(maybe_root_from_subdir.is_some(), "Should detect git repo from subdir");
         assert_eq!(maybe_root_from_subdir.unwrap().canonicalize()?, path.canonicalize()?);

        Ok(())
    }

    #[test]
    fn test_detect_git_repo_outside() -> anyhow::Result<()> {
        let (_dir, path) = setup_test_dir()?; // Just a plain directory

        let maybe_root = detect_git_repo(&path)?;
        assert!(maybe_root.is_none(), "Should not detect git repo");
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_no_exclude() -> anyhow::Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false, // N/A for walkdir
        };

        let files = list_files_walkdir(&path, &config)?;

        // Expecting: file1.txt, file2.rs, subdir/file3.log, subdir/another.txt, binary.dat
        // Order might vary, so use a HashSet or sort for comparison
        let expected_files: Vec<PathBuf> = [
            "file1.txt",
            "file2.rs",
            "subdir/file3.log",
            "subdir/another.txt",
            "binary.dat" // walkdir doesn't know about .gitignore
        ]
        .iter()
        .map(|f| path.join(f))
        .collect();

        // Simple count check for now, more robust check needed for actual paths
        assert_eq!(files.len(), expected_files.len());
        // TODO: Improve check to be order-independent (e.g., sort paths or use HashSet)
        println!("Walkdir found files: {:?}", files);

        Ok(())
    }

     #[test]
     fn test_list_files_walkdir_with_exclude() -> anyhow::Result<()> {
         let (_dir, path) = setup_test_dir()?;
         let config = GrabConfig {
             target_path: path.clone(),
             add_headers: false,
             exclude_patterns: vec!["*.log".to_string(), "subdir/".to_string()], // Exclude logs and subdir content
             include_untracked: false,
         };

         let files = list_files_walkdir(&path, &config)?;

         // Expecting: file1.txt, file2.rs, binary.dat
         // subdir/* and *.log should be excluded
         let expected_files: Vec<PathBuf> = [
             "file1.txt",
             "file2.rs",
             "binary.dat"
         ]
         .iter()
         .map(|f| path.join(f))
         .collect();

         assert_eq!(files.len(), expected_files.len());
         println!("Walkdir (with exclude) found files: {:?}", files);
         // TODO: Improve check to be order-independent

         Ok(())
     }

    #[test]
    fn test_process_files_no_headers_skip_binary() -> anyhow::Result<()> {
         let (_dir, path) = setup_test_dir()?;
         let files_to_process = vec![
             path.join("file1.txt"),
             path.join("binary.dat"), // Should be skipped
             path.join("file2.rs"),
         ];

         let result = process_files(&files_to_process, false, None)?; // No headers, repo_root doesn't matter

         let expected_content = "Content of file 1.\n\nfn main() {}\n\n"; // Note the double newlines

         assert_eq!(result.trim(), expected_content.trim()); // Use trim for flexibility
         // TODO: Check log output for the warning about skipping binary.dat (requires logger setup in tests)

         Ok(())
    }

     #[test]
     fn test_process_files_with_headers() -> anyhow::Result<()> {
         let (_dir, path) = setup_test_dir()?;
         let files_to_process = vec![
             path.join("file1.txt"),
             path.join("file2.rs"),
         ];

         // Simulate being in repo root for cleaner header paths
         let repo_root = Some(path.as_path());

         let result = process_files(&files_to_process, true, repo_root)?;

         let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nfn main() {{}}\n\n",
            Path::new("file1.txt").display(), // Relative paths
            Path::new("file2.rs").display()
         );

         assert_eq!(result.trim(), expected_content.trim());

         Ok(())
     }

    // TODO: Add tests for list_files_git (tracked, untracked, ignored, exclude patterns)
    // These are more involved as they require verifying git commands ran correctly.

} // End of mod tests