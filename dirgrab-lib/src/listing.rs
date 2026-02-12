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
            let root_path_raw = PathBuf::from(&stdout);
            let root_path = root_path_raw
                .canonicalize()
                .map_err(|e| GrabError::IoError {
                    path: root_path_raw.clone(),
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
        if stderr.contains("fatal: detected dubious ownership in repository at") {
            warn!(
                "Git reports 'dubious ownership' for {:?}. Falling back to non-git mode. Consider running: git config --global --add safe.directory {:?}",
                path, path
            );
            Ok(None)
        } else if stderr.contains("not a git repository") {
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
pub(crate) fn list_files_git(
    repo_root: &Path,
    config: &GrabConfig,
    scope_subdir: Option<&Path>,
) -> GrabResult<Vec<PathBuf>> {
    debug!(
        "Listing files using Git in root {:?} with scope {:?}",
        repo_root, scope_subdir
    );

    let mut combined_files = HashSet::new();

    let scope_specs = build_scope_pathspecs(repo_root, scope_subdir);
    let exclude_specs = build_exclude_pathspecs(config);

    let mut tracked_args = vec!["ls-files".to_string(), "-z".to_string()];
    tracked_args.extend(scope_specs.iter().cloned());
    tracked_args.extend(exclude_specs.iter().cloned());

    run_git_ls(repo_root, &tracked_args, "tracked", &mut combined_files)?;

    if config.include_untracked {
        let mut untracked_args = vec![
            "ls-files".to_string(),
            "-z".to_string(),
            "--others".to_string(),
            "--exclude-standard".to_string(),
        ];
        untracked_args.extend(scope_specs.iter().cloned());
        untracked_args.extend(exclude_specs.iter().cloned());

        run_git_ls(repo_root, &untracked_args, "untracked", &mut combined_files)?;
    } else {
        debug!("Skipping untracked files per configuration.");
    }

    let mut files: Vec<PathBuf> = combined_files.into_iter().collect();
    files.sort();
    Ok(files)
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
        let pattern = normalize_glob("dirgrab.txt");
        if let Err(e) = exclude_builder.add_line(None, &pattern) {
            warn!("Failed to add default exclusion pattern 'dirgrab.txt': {}. This exclusion might not apply.", e);
        } else {
            debug!("Applying default exclusion for 'dirgrab.txt'");
        }
    } else {
        info!("Default exclusion for 'dirgrab.txt' is disabled by --include-default-output flag.");
    }
    // Always exclude the .git directory when using walkdir
    let git_dir_pattern = normalize_glob(".git/");
    if let Err(e) = exclude_builder.add_line(None, &git_dir_pattern) {
        warn!(
            "Failed to add default exclusion pattern '.git/': {}. Git directory might be included.",
            e
        );
    } else {
        debug!("Applying default exclusion for '.git/'");
    }

    // Add user-provided exclusion patterns
    for pattern in &config.exclude_patterns {
        let normalized = normalize_glob(pattern);
        if let Err(e) = exclude_builder.add_line(None, &normalized) {
            error!(
                "Failed to add exclude pattern '{}': {}. This pattern will be ignored.",
                pattern, e
            );
        }
    }
    let exclude_matcher = exclude_builder
        .build()
        .map_err(GrabError::GlobMatcherBuildError)?;

    // Canonicalize the target to use as a boundary check for symlinks.
    let canonical_root = target_path
        .canonicalize()
        .unwrap_or_else(|_| target_path.to_path_buf());

    // Walk directory while pruning ignored subtrees early.
    // follow_links(true) matches Git mode behavior where symlinked files are included.
    // Walkdir detects circular symlinks and emits errors, which we handle below.
    let mut walker = WalkDir::new(target_path).follow_links(true).into_iter();
    while let Some(entry_result) = walker.next() {
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

        // Boundary check: if a symlink resolves outside the target directory, skip it.
        if entry.path_is_symlink() {
            if let Ok(canonical) = path.canonicalize() {
                if !canonical.starts_with(&canonical_root) {
                    debug!(
                        "Skipping symlink that escapes target directory: {:?} -> {:?}",
                        path, canonical
                    );
                    if entry.file_type().is_dir() {
                        walker.skip_current_dir();
                    }
                    continue;
                }
            }
        }

        if entry.file_type().is_dir() {
            if matches!(
                exclude_matcher.matched_path_or_any_parents(path, true),
                Match::Ignore(_)
            ) {
                debug!(
                    "Pruning directory due to pattern match on path or parent (walkdir): {:?}",
                    path
                );
                walker.skip_current_dir();
            }
            continue;
        }

        if !entry.file_type().is_file() {
            continue;
        }

        match exclude_matcher.matched_path_or_any_parents(path, false) {
            Match::None | Match::Whitelist(_) => {
                files.push(path.to_path_buf());
            }
            Match::Ignore(_) => {
                debug!(
                    "Excluding file due to pattern match on path or parent (walkdir): {:?}",
                    path
                );
            }
        }
    }

    files.sort();
    Ok(files)
}

fn run_git_ls(
    repo_root: &Path,
    args: &[String],
    phase: &str,
    combined_files: &mut HashSet<PathBuf>,
) -> GrabResult<()> {
    let display_command = format!("git {}", args.join(" "));
    debug!(
        "Running git command for {} files: {}",
        phase, display_command
    );

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let output = run_command("git", &arg_refs, repo_root)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        error!(
            "git ls-files command ({}) failed.\nStderr: {}\nStdout: {}",
            phase, stderr, stdout
        );
        return Err(GrabError::GitCommandError {
            command: display_command,
            stderr,
            stdout,
        });
    }

    for path in String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
    {
        combined_files.insert(repo_root.join(path));
    }

    Ok(())
}

fn build_scope_pathspecs(repo_root: &Path, scope_subdir: Option<&Path>) -> Vec<String> {
    let mut specs = Vec::new();
    if let Some(rel_path) = scope_subdir {
        if rel_path.as_os_str().is_empty() {
            return specs;
        }

        let absolute_path = repo_root.join(rel_path);
        let normalized = normalize_for_git(rel_path);
        if absolute_path.is_dir() {
            let suffix = if normalized.ends_with('/') {
                "**"
            } else {
                "/**"
            };
            let spec = format!(":(glob){}{}", normalized, suffix);
            specs.push(spec);
        } else {
            specs.push(format!(":(glob){}", normalized));
        }
    }
    specs
}

fn build_exclude_pathspecs(config: &GrabConfig) -> Vec<String> {
    let mut specs = Vec::new();
    let mut seen = HashSet::new();

    if !config.include_default_output {
        let normalized = normalize_glob("dirgrab.txt");
        if seen.insert(normalized.clone()) {
            debug!("Applying default exclusion for 'dirgrab.txt'");
            specs.push(format!(":(glob,exclude){}", prefix_for_git(&normalized)));
        }
    } else {
        info!("Default exclusion for 'dirgrab.txt' is disabled by configuration.");
    }

    for pattern in &config.exclude_patterns {
        let normalized = normalize_glob(pattern);
        if seen.insert(normalized.clone()) {
            specs.push(format!(":(glob,exclude){}", prefix_for_git(&normalized)));
        } else {
            debug!(
                "Skipping duplicate exclude pattern '{}' when building git pathspecs",
                pattern
            );
        }
    }

    specs
}

fn normalize_for_git(path: &Path) -> String {
    path.components()
        .map(|comp| comp.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn prefix_for_git(pattern: &str) -> String {
    if pattern.contains('/') {
        pattern.to_string()
    } else {
        format!("**/{}", pattern)
    }
}

/// Normalizes a glob pattern by replacing backslashes with forward slashes.
/// Used to ensure consistent pattern matching across platforms.
pub fn normalize_glob(pattern: &str) -> String {
    pattern.replace('\\', "/")
}
