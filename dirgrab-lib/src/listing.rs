// --- FILE: dirgrab-lib/src/listing.rs ---

use std::collections::HashSet; // Needed for list_files_git
use std::io; // Needed for io::ErrorKind::NotFound check indirectly via run_command/detect_git_repo
use std::path::{Path, PathBuf};

use ignore::gitignore::GitignoreBuilder;
use ignore::Match;
use log::{debug, error, info, warn};
use walkdir::WalkDir;

// Use crate:: paths for sibling modules
use crate::config::GrabConfig;
use crate::errors::{GrabError, GrabResult};
use crate::utils::run_command; // Use the utility function

/// Checks if the path is inside a Git repository and returns the repo root if true.
/// Crate-public as it's only called by grab_contents in lib.rs.
pub(crate) fn detect_git_repo(path: &Path) -> GrabResult<Option<PathBuf>> {
    let command_str = "git rev-parse --show-toplevel";
    debug!(
        "Detecting git repo by running '{}' in path: {:?}",
        command_str, path
    );

    // Attempt to run git command, handle specific "not found" error gracefully
    let output = match run_command("git", &["rev-parse", "--show-toplevel"], path) {
        // Uses run_command
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
/// Crate-public as it's only called by grab_contents in lib.rs.
pub(crate) fn list_files_git(repo_root: &Path, config: &GrabConfig) -> GrabResult<Vec<PathBuf>> {
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

    let mut combined_files = HashSet::new(); // Uses HashSet

    // Tracked files
    let mut tracked_args = base_args.to_vec();
    tracked_args.extend_from_slice(&all_exclude_refs);
    let tracked_command_str = format!("git {}", tracked_args.join(" "));
    debug!(
        "Running git command for tracked files: {}",
        tracked_command_str
    );
    let tracked_output = run_command("git", &tracked_args, repo_root)?; // Uses run_command
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
        let untracked_output = run_command("git", &untracked_args, repo_root)?; // Uses run_command
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
/// Crate-public as it's only called by grab_contents in lib.rs.
pub(crate) fn list_files_walkdir(
    target_path: &Path,
    config: &GrabConfig,
) -> GrabResult<Vec<PathBuf>> {
    debug!("Listing files using walkdir starting at: {:?}", target_path);
    let mut files = Vec::new();

    let mut exclude_builder = GitignoreBuilder::new(target_path);

    // Add default exclusions for dirgrab.txt (conditionally) and .git/
    if !config.include_default_output {
        if let Err(e) = exclude_builder.add_line(None, "dirgrab.txt") {
            warn!("Failed to add default exclusion pattern 'dirgrab.txt': {}. This exclusion might not apply.", e);
        } else {
            debug!("Applying default exclusion for 'dirgrab.txt'");
        }
    } else {
        info!("Default exclusion for 'dirgrab.txt' is disabled by --include-default-output flag.");
    }
    // Always exclude the .git directory when using walkdir
    if let Err(e) = exclude_builder.add_line(None, ".git/") {
        warn!(
            "Failed to add default exclusion pattern '.git/': {}. Git directory might be included.",
            e
        );
    } else {
        debug!("Applying default exclusion for '.git/'");
    }

    // Add user-provided exclusion patterns
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

    // Use WalkDir with the custom matcher applied via filtering
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

        // Skip non-files early
        if !entry.file_type().is_file() {
            continue;
        }

        // Apply exclusion rules using the patterns.
        // Use matched_path_or_any_parents to correctly handle directory exclusions like "subdir/"
        match exclude_matcher.matched_path_or_any_parents(path, false) {
            Match::None | Match::Whitelist(_) => {
                // Not ignored, add it
                files.push(path.to_path_buf());
            }
            Match::Ignore(_) => {
                // Ignored by a pattern (could be the path itself or a parent dir)
                debug!(
                    "Excluding file due to pattern match on path or parent (walkdir): {:?}",
                    path
                );
                continue; // Skip this file
            }
        }
    }

    Ok(files)
}
