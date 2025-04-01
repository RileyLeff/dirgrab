use std::fs;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::collections::HashSet;
use thiserror::Error;
use walkdir::WalkDir;
use ignore::gitignore::{GitignoreBuilder};
use ignore::Match;

// Re-export log macros for convenience
use log::{debug, info, warn, error};

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
    GitExecutionError{
        command: String,
        #[source] source: io::Error
    },
    #[error("Failed to read non-UTF8 file: {0}")]
    NonUtf8File(PathBuf),
    #[error("Could not determine repository root for: {0}")]
    RepoRootNotFound(PathBuf),
    #[error("Failed to build glob pattern matcher: {0}")]
    GlobMatcherBuildError(#[source] ignore::Error),
    #[error("Error walking directory {path_display}: {source_str}")]
    WalkdirError {
         path_display: String,
         source_str: String,
    }
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

    let target_path = config.target_path.canonicalize().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            GrabError::TargetPathNotFound(config.target_path.clone())
        } else {
            GrabError::IoError { path: config.target_path.clone(), source: e }
        }
    })?;
    debug!("Canonical target path: {:?}", target_path);

    let git_repo_root = detect_git_repo(&target_path)?;

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
        return Ok(String::new());
    }

    process_files(&files_to_process, config.add_headers, git_repo_root.as_deref())
}

// --- Helper Function Implementations ---

/// Checks if the path is inside a Git repository and returns the repo root if true.
fn detect_git_repo(path: &Path) -> GrabResult<Option<PathBuf>> {
    let command_str = "git rev-parse --show-toplevel";
    debug!("Detecting git repo by running '{}' in path: {:?}", command_str, path);

    let output = match run_command("git", &["rev-parse", "--show-toplevel"], path) {
        Ok(output) => output,
        Err(GrabError::GitExecutionError{ source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            // Git command not found, definitely not a Git repo context for us
            info!("'git' command not found. Assuming Non-Git mode.");
            return Ok(None);
        }
        Err(e) => return Err(e), // Propagate other execution errors
    };


    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
         if !stdout.is_empty() {
            let root_path = PathBuf::from(stdout).canonicalize().map_err(|e| GrabError::IoError { path: PathBuf::from("detected git root"), source: e})?;
             debug!("Detected Git repo root: {:?}", root_path);
             Ok(Some(root_path))
         } else {
             warn!("'{}' succeeded but returned empty output in {:?}", command_str, path);
             Ok(None)
         }
    } else {
         let stderr = String::from_utf8_lossy(&output.stderr);
         if stderr.contains("not a git repository") || stderr.contains("fatal: detected dubious ownership in repository at") {
             debug!("Path is not inside a Git repository (based on stderr): {:?}", path);
             Ok(None)
         } else {
             let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
             error!("Git command '{}' failed unexpectedly.\nStderr: {}\nStdout: {}", command_str, stderr, stdout);
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
    debug!("Listing files using Git in root: {:?}", repo_root);

    // --- Build base arguments and exclusions ---
    let base_args = ["ls-files", "-z"]; // Always use -z
    let exclude_pathspecs: Vec<String> = config.exclude_patterns.iter()
        .map(|p| format!(":!{}", p))
        .collect();
    let exclude_refs: Vec<&str> = exclude_pathspecs.iter().map(AsRef::as_ref).collect();

    // --- Collect files based on config ---
    let mut combined_files = HashSet::new(); // Use HashSet for automatic deduplication

    // 1. Get TRACKED files
    let mut tracked_args = base_args.to_vec();
    tracked_args.extend_from_slice(&exclude_refs); // Apply exclusions to tracked files
    let tracked_command_str = format!("git {}", tracked_args.join(" "));
    debug!("Running git command for tracked files: {}", tracked_command_str);
    let tracked_output = run_command("git", &tracked_args, repo_root)?;
    if !tracked_output.status.success() {
        // Handle error same way as before...
        let stderr = String::from_utf8_lossy(&tracked_output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&tracked_output.stdout).into_owned();
        error!("git ls-files command (tracked) failed.\nStderr: {}\nStdout: {}", stderr, stdout);
        return Err(GrabError::GitCommandError { command: tracked_command_str, stderr, stdout });
    }
    String::from_utf8_lossy(&tracked_output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .for_each(|s| { combined_files.insert(repo_root.join(s)); });


    // 2. Get UNTRACKED files (if requested)
    if config.include_untracked {
        let mut untracked_args = base_args.to_vec();
        untracked_args.push("--others");
        untracked_args.push("--exclude-standard"); // Respect .gitignore for untracked
        untracked_args.extend_from_slice(&exclude_refs); // Apply exclusions to untracked files too
        let untracked_command_str = format!("git {}", untracked_args.join(" "));
        debug!("Running git command for untracked files: {}", untracked_command_str);
        let untracked_output = run_command("git", &untracked_args, repo_root)?;

        if !untracked_output.status.success() {
             // Handle error same way as before...
            let stderr = String::from_utf8_lossy(&untracked_output.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&untracked_output.stdout).into_owned();
            error!("git ls-files command (untracked) failed.\nStderr: {}\nStdout: {}", stderr, stdout);
            return Err(GrabError::GitCommandError { command: untracked_command_str, stderr, stdout });
        }
         String::from_utf8_lossy(&untracked_output.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .for_each(|s| { combined_files.insert(repo_root.join(s)); });
    }

    // Convert HashSet back to Vec
    let files_vec = combined_files.into_iter().collect();
    Ok(files_vec)
}


/// Lists files using `walkdir`.
fn list_files_walkdir(target_path: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using walkdir starting at: {:?}", target_path);
    let mut files = Vec::new();

    let mut exclude_builder = GitignoreBuilder::new(target_path);
    for pattern in &config.exclude_patterns {
        if let Err(e) = exclude_builder.add_line(None, pattern) {
             error!("Failed to add exclude pattern '{}': {}. This pattern will be ignored.", pattern, e);
        }
    }
    let exclude_matcher = exclude_builder.build().map_err(GrabError::GlobMatcherBuildError)?;

    for entry_result in WalkDir::new(target_path) {
        let entry = match entry_result {
            Ok(entry) => entry,
            Err(e) => {
                let path_display = e.path().map_or_else(|| target_path.display().to_string(), |p| p.display().to_string());
                warn!("Skipping path due to error during walk near {}: {}", path_display, e);
                 continue;
            }
        };

        let path = entry.path();

        if !entry.file_type().is_file() {
            continue;
        }

        // --- Apply exclusion rules ---
        // Use `matched_path_or_any_parents` to correctly handle directory exclusions (e.g., `target/`)
        // applied to files within those directories.
        // `is_dir` is `false` here since we already filtered for files.
        match exclude_matcher.matched_path_or_any_parents(path, false) { // <--- The changed line
            Match::None | Match::Whitelist(_) => {
                // Not ignored by --exclude patterns, add it
                files.push(path.to_path_buf());
                // debug!("Including file (walkdir): {:?}", path);
            }
            Match::Ignore(_) => {
                // Ignored by --exclude pattern (either directly or via parent dir)
                debug!("Excluding file due to pattern (walkdir): {:?} matching pattern for path or parent", path);
                continue; // Skip this file
            }
        }
    } // End walkdir loop

    Ok(files)
}


/// Reads a list of files and concatenates their content.
fn process_files(files: &[PathBuf], add_headers: bool, repo_root: Option<&Path>) -> GrabResult<String> {
    debug!("Processing {} files.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024);
    let mut buffer = Vec::new();

    for file_path in files {
        debug!("Processing file: {:?}", file_path);

        if add_headers {
            let display_path = repo_root
                .and_then(|root| file_path.strip_prefix(root).ok())
                .unwrap_or(file_path);

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
                                warn!("Skipping non-UTF8 file: {:?}", file_path);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Skipping file due to read error: {:?} - {}", file_path, e);
                    }
                }
            }
            Err(e) => {
                warn!("Skipping file due to open error: {:?} - {}", file_path, e);
            }
        }
    }

    Ok(combined_content)
}

/// Utility function to run a command and capture output
fn run_command(cmd: &str, args: &[&str], current_dir: &Path) -> GrabResult<Output> {
    debug!("Running command: {} {:?} in directory: {:?}", cmd, args, current_dir);
    let output = Command::new(cmd)
        .args(args)
        .current_dir(current_dir)
        .output()
        .map_err(|e| {
             let command_string = format!("{} {}", cmd, args.join(" "));
             if e.kind() == std::io::ErrorKind::NotFound {
                 error!("Command '{}' not found. Is '{}' installed and in your system's PATH?", command_string, cmd);
             }
             GrabError::GitExecutionError{ command: command_string, source: e }
        })?;

    Ok(output)
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet; // For order-independent comparison
    use std::fs::{self};
    use std::path::Path; // Ensure Path is imported directly for test cases
    use std::process::Command;
    use tempfile::{tempdir, TempDir};
    use anyhow::Result;

    // Helper function to create a basic temporary directory setup
    fn setup_test_dir() -> Result<(TempDir, PathBuf)> {
        let dir = tempdir()?;
        let path = dir.path().to_path_buf();

        fs::write(path.join("file1.txt"), "Content of file 1.")?;
        fs::write(path.join("file2.rs"), "fn main() {}")?;
        fs::create_dir(path.join("subdir"))?;
        fs::write(path.join("subdir").join("file3.log"), "Log message.")?;
        fs::write(path.join("subdir").join("another.txt"), "Another text file.")?;
        fs::write(path.join("binary.dat"), &[0x80, 0x81, 0x82])?;

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

        run_command_test("git", &["add", ".gitignore", "file2.rs", "subdir/another.txt"], path)?; // Add specific files + .gitignore
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
        println!("Running test command: {} {:?} in {:?}", cmd, args, current_dir);
        let output = Command::new(cmd)
            .args(args)
            .current_dir(current_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
             anyhow::bail!(
                 "Command failed: {} {:?}\nStatus: {}\nStdout: {}\nStderr: {}",
                 cmd, args, output.status, stdout, stderr
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
        if !setup_git_repo(&path)? { return Ok(()); } // Skip if git not available

        let maybe_root = detect_git_repo(&path)?;
        assert!(maybe_root.is_some(), "Should detect git repo");
        assert_eq!(maybe_root.unwrap().canonicalize()?, path.canonicalize()?);

        let subdir_path = path.join("subdir");
         let maybe_root_from_subdir = detect_git_repo(&subdir_path)?;
         assert!(maybe_root_from_subdir.is_some(), "Should detect git repo from subdir");
         assert_eq!(maybe_root_from_subdir.unwrap().canonicalize()?, path.canonicalize()?);

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

        let expected_set = get_expected_set(&path, &[
            "file1.txt",
            "file2.rs",
            "subdir/file3.log",
            "subdir/another.txt",
            "binary.dat"
        ]);
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

        let expected_set = get_expected_set(&path, &[
            "file1.txt",
            "file2.rs",
            "binary.dat"
        ]);
        assert_paths_eq(files, expected_set);
        Ok(())
     }

    // --- NEW Git Tests ---

    #[test]
    fn test_list_files_git_tracked_only() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? { return Ok(()); } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(), // Not directly used by list_files_git, but needed
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false, // Default: only tracked files
        };

        let files = list_files_git(&path, &config)?;

        // Expected: Only files explicitly added and committed (.gitignore, file2.rs, subdir/another.txt)
        let expected_set = get_expected_set(&path, &[
            ".gitignore",
            "file2.rs",
            "subdir/another.txt",
        ]);

        println!("Git tracked files found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_untracked() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? { return Ok(()); } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true, // The key flag for this test
        };

        let files = list_files_git(&path, &config)?;

        // Expected: tracked files + untracked.txt
        // .gitignore'd files (file1.txt, binary.dat, *.log) should NOT be included
        let expected_set = get_expected_set(&path, &[
            ".gitignore",
            "file2.rs",
            "subdir/another.txt",
            "untracked.txt", // The untracked file
        ]);

        println!("Git tracked+untracked files found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? { return Ok(()); } // Skip if git not available

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            // Exclude Rust files and everything in subdir/
            exclude_patterns: vec!["*.rs".to_string(), "subdir/".to_string()],
            include_untracked: false, // Tracked only
        };

        let files = list_files_git(&path, &config)?;

        // Expected: .gitignore (file2.rs and subdir/another.txt are excluded)
        let expected_set = get_expected_set(&path, &[
            ".gitignore",
        ]);

        println!("Git tracked files (with exclude) found: {:?}", files);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

     #[test]
     fn test_list_files_git_untracked_with_exclude() -> Result<()> {
         let (_dir, path) = setup_test_dir()?;
         if !setup_git_repo(&path)? { return Ok(()); } // Skip if git not available

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
         let expected_set = get_expected_set(&path, &[
             ".gitignore",
             "file2.rs",
         ]);

         println!("Git tracked+untracked (with exclude) files found: {:?}", files);
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
         let files_to_process = vec![
             path.join("file1.txt"),
             path.join("file2.rs"),
         ];

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