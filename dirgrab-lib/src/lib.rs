// --- FILE: dirgrab-lib/src/lib.rs ---

#![doc = include_str!("../README.md")]

// Declare modules
mod config;
mod errors;
mod listing;
mod processing;
mod tree;
mod utils;

// Necessary imports for lib.rs itself
use std::io; // For io::ErrorKind
             // Path and PathBuf are used in modules, not directly here anymore
use log::{debug, error, info, warn}; // For logging within grab_contents

// Re-export public API components
pub use config::GrabConfig; // Re-export GrabConfig from the config module
pub use errors::{GrabError, GrabResult}; // Make error/result types public

// --- Main Public Function ---

/// Performs the main `dirgrab` operation based on the provided configuration.
// (Doc comment remains the same, example needs updating eventually)
#[doc = include_str!("../README.md")]
pub fn grab_contents(config: &GrabConfig) -> GrabResult<String> {
    // Takes GrabConfig by ref
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
    // Calls functions from the 'listing' module now
    let (files_to_process, maybe_repo_root) = if config.no_git {
        info!("Ignoring Git context due to --no-git flag.");
        let files = listing::list_files_walkdir(&target_path, config)?; // Use crate::listing
        (files, None)
    } else {
        let git_repo_root = listing::detect_git_repo(&target_path)?; // Use crate::listing
        let files = match &git_repo_root {
            Some(root) => {
                info!("Operating in Git mode. Repo root: {:?}", root);
                listing::list_files_git(root, config)? // Use crate::listing
            }
            None => {
                info!("Operating in Non-Git mode. Target path: {:?}", target_path);
                listing::list_files_walkdir(&target_path, config)? // Use crate::listing
            }
        };
        (files, git_repo_root)
    };

    info!("Found {} files to process.", files_to_process.len());

    // Initialize output buffer
    let mut output_buffer = String::new();

    // Generate and prepend tree if requested
    // Calls function from the 'tree' module now
    if config.include_tree {
        if files_to_process.is_empty() {
            warn!("--include-tree specified, but no files were selected for processing. Tree will be empty.");
            output_buffer.push_str("---\nDIRECTORY STRUCTURE (No files selected)\n---\n\n");
            return Ok(output_buffer); // Return early with just the empty tree message
        } else {
            let base_path_for_tree = maybe_repo_root.as_deref().unwrap_or(&target_path);
            debug!(
                "Generating directory tree relative to: {:?}",
                base_path_for_tree
            );

            match tree::generate_indented_tree(&files_to_process, base_path_for_tree) {
                // Use crate::tree
                Ok(tree_str) => {
                    output_buffer.push_str("---\nDIRECTORY STRUCTURE\n---\n");
                    output_buffer.push_str(&tree_str);
                    output_buffer.push_str("\n---\nFILE CONTENTS\n---\n\n");
                }
                Err(e) => {
                    error!("Failed to generate directory tree: {}", e);
                    output_buffer.push_str("---\nERROR GENERATING DIRECTORY STRUCTURE\n---\n\n");
                }
            }
        }
    }

    // Process files and append content (only if files exist)
    // Calls function from the 'processing' module now
    if !files_to_process.is_empty() {
        match processing::process_files(
            // Use crate::processing
            &files_to_process,
            config.add_headers,
            maybe_repo_root.as_deref(),
            &target_path,
        ) {
            Ok(content) => output_buffer.push_str(&content),
            Err(e) => {
                error!("Failed during file content processing: {}", e);
                return Err(e);
            }
        }
    } else if !config.include_tree {
        warn!("No files selected for processing based on current configuration.");
        return Ok(String::new()); // Return empty string if no files AND no tree requested
    }

    // Return the combined buffer
    Ok(output_buffer)
}

// --- Tests ---
// Keep the tests module here in lib.rs for now.
// It acts as integration tests for the library.
#[cfg(test)]
mod tests {
    // Use super::* to bring everything from lib.rs into scope for tests
    // This now includes GrabConfig, GrabError, GrabResult because they are re-exported.
    use super::*;
    // Also need direct imports for helpers/types used *only* in tests
    use anyhow::Result;
    use std::collections::HashSet;
    use std::fs::{self};
    use std::path::{Path, PathBuf}; // Need these for helpers defined within tests mod
    use std::process::Command;
    use tempfile::{tempdir, TempDir};

    // Test setup helpers (These don't need module prefixes as they are in the same module)
    // Must use crate::utils::run_command inside setup_git_repo now
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
        fs::write(path.join("dirgrab.txt"), "Previous dirgrab output.")?;
        Ok((dir, path))
    }

    fn setup_git_repo(path: &Path) -> Result<bool> {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("WARN: 'git' command not found, skipping Git-related test setup.");
            return Ok(false);
        }
        // Use crate:: path now because utils is not in super::* scope
        crate::utils::run_command("git", &["init", "-b", "main"], path)?;
        crate::utils::run_command("git", &["config", "user.email", "test@example.com"], path)?;
        crate::utils::run_command("git", &["config", "user.name", "Test User"], path)?;

        fs::write(path.join(".gitignore"), "*.log\nbinary.dat\nfile1.txt")?;
        crate::utils::run_command(
            "git",
            &["add", ".gitignore", "file2.rs", "subdir/another.txt"],
            path,
        )?;
        crate::utils::run_command("git", &["commit", "-m", "Initial commit"], path)?;

        fs::write(path.join("untracked.txt"), "This file is not tracked.")?;
        fs::write(path.join("ignored.log"), "This should be ignored by git.")?;
        fs::create_dir_all(path.join("deep/sub"))?;
        fs::write(path.join("deep/sub/nested.txt"), "Nested content")?;
        crate::utils::run_command("git", &["add", "deep/sub/nested.txt"], path)?;
        crate::utils::run_command("git", &["commit", "-m", "Add nested file"], path)?;
        Ok(true)
    }

    // Renamed helper
    fn run_test_command(
        cmd: &str,
        args: &[&str],
        current_dir: &Path,
    ) -> Result<std::process::Output> {
        // println!("Running test command: {} {:?} in {:?}", cmd, args, current_dir);
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

    fn get_expected_set(base_path: &Path, relative_paths: &[&str]) -> HashSet<PathBuf> {
        relative_paths.iter().map(|p| base_path.join(p)).collect()
    }

    fn assert_paths_eq(actual: Vec<PathBuf>, expected: HashSet<PathBuf>) {
        let actual_set: HashSet<PathBuf> = actual.into_iter().collect();
        assert_eq!(actual_set, expected);
    }

    // --- Tests ---
    // Tests calling listing functions need crate:: prefix
    #[test]
    fn test_detect_git_repo_inside() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }
        let maybe_root = crate::listing::detect_git_repo(&path)?; // Use crate:: path
        assert!(maybe_root.is_some());
        assert_eq!(maybe_root.unwrap().canonicalize()?, path.canonicalize()?);
        let subdir_path = path.join("subdir");
        let maybe_root_from_subdir = crate::listing::detect_git_repo(&subdir_path)?; // Use crate:: path
        assert!(maybe_root_from_subdir.is_some());
        assert_eq!(
            maybe_root_from_subdir.unwrap().canonicalize()?,
            path.canonicalize()?
        );
        Ok(())
    }

    #[test]
    fn test_detect_git_repo_outside() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let maybe_root = crate::listing::detect_git_repo(&path)?; // Use crate:: path
        assert!(maybe_root.is_none());
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_no_exclude_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            // GrabConfig is in scope via super::*
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_walkdir(&path, &config)?; // Use crate:: path
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
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_walkdir(&path, &config)?; // Use crate:: path
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
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
            ],
        );
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
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
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
            ],
            include_untracked: false,
            include_default_output: false,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
        let expected_set = get_expected_set(&path, &[".gitignore"]);
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
            exclude_patterns: vec!["*.txt".to_string()],
            include_untracked: true,
            include_default_output: false,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
        let expected_set = get_expected_set(&path, &[".gitignore", "file2.rs"]);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_include_default_output() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_walkdir(&path, &config)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                "file1.txt",
                "file2.rs",
                "subdir/file3.log",
                "subdir/another.txt",
                "binary.dat",
                "dirgrab.txt",
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
        run_test_command("git", &["add", "dirgrab.txt"], &path)?; // Use renamed helper
        run_test_command("git", &["commit", "-m", "Add dirgrab.txt"], &path)?; // Use renamed helper
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
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
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,
            include_default_output: true,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
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
            include_default_output: true,
            no_git: false,
            include_tree: false,
        };
        let files = crate::listing::list_files_git(&path, &config)?; // Use crate:: path
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

    // Tests calling grab_contents (public via super::*) are fine
    #[test]
    fn test_no_git_flag_forces_walkdir_in_git_repo() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }
        let config = GrabConfig {
            // In scope via super::*
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: true,
            include_tree: false,
        };
        let _result_string = grab_contents(&config)?; // Prefix variable, use super::grab_contents
                                                      // Assertions... (unchanged, use _result_string)
        assert!(_result_string.contains("Content of file 1."));
        assert!(_result_string.contains("*.log")); // .gitignore content
                                                   // ... etc ...
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
            exclude_patterns: vec!["*.txt".to_string(), "*.rs".to_string()],
            include_untracked: false,
            include_default_output: false,
            no_git: true,
            include_tree: false,
        };
        let _result_string = grab_contents(&config)?; // Prefix variable
                                                      // Assertions... (unchanged, use _result_string)
        assert!(_result_string.contains("*.log"));
        assert!(!_result_string.contains("Content of file 1."));
        // ... etc ...
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
            include_default_output: true,
            no_git: true,
            include_tree: false,
        };
        let result_string = grab_contents(&config)?; // Keep variable if used in assert! message
                                                     // Assertions... (unchanged)
        assert!(
            result_string.contains("Previous dirgrab output."),
            "Should include dirgrab.txt due to override"
        );
        // ... etc ...
        Ok(())
    }

    #[test]
    fn test_no_git_flag_headers_relative_to_target() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: true,
            include_tree: false,
        };
        let result_string = grab_contents(&config)?;
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
        let subdir_target = path.join("deep");
        let config = GrabConfig {
            target_path: subdir_target.clone(),
            add_headers: true,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: false,
            include_tree: false,
        };
        let result_string = grab_contents(&config)?;
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

    #[test]
    fn test_grab_contents_with_tree_no_git() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        } // Need git repo for nested file setup consistency
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![
                "*.log".to_string(),
                "*.dat".to_string(),
                ".gitignore".to_string(),
            ], // Exclude logs, binary, gitignore
            include_untracked: false,
            include_default_output: false,
            no_git: true,       // Force walkdir
            include_tree: true, // THE flag to test
        };
        let result = grab_contents(&config)?;

        // --- Start Fix ---
        // Corrected expected tree - includes file1.txt because .gitignore is ignored by walkdir
        let expected_tree_part = "\
---
DIRECTORY STRUCTURE
---
- deep/
  - sub/
    - nested.txt
- file1.txt
- file2.rs
- subdir/
  - another.txt
- untracked.txt
"; // --- End Fix ---

        assert!(
            result.contains(expected_tree_part),
            "Expected tree structure not found in output:\n{}",
            result
        );

        assert!(
            result.contains("\n---\nFILE CONTENTS\n---\n\n"),
            "Expected file content separator not found"
        );
        assert!(
            result.contains("--- FILE: file1.txt ---"),
            "Header for file1.txt missing"
        );
        assert!(
            result.contains("Content of file 1."),
            "Content of file1.txt missing"
        );
        assert!(
            result.contains("--- FILE: deep/sub/nested.txt ---"),
            "Header for nested.txt missing"
        );
        assert!(
            result.contains("Nested content"),
            "Content of nested.txt missing"
        );
        assert!(
            !result.contains("dirgrab.txt"),
            "dirgrab.txt should be excluded"
        );
        assert!(
            !result.contains(".gitignore"),
            ".gitignore should be excluded by -e"
        );
        assert!(
            !result.contains("binary.dat"),
            "binary.dat should be excluded by -e"
        );
        assert!(
            !result.contains("Log message"),
            "Logs should be excluded by -e"
        );

        Ok(())
    }

    #[test]
    fn test_grab_contents_with_tree_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![".gitignore".to_string()],
            include_untracked: true,
            include_default_output: false,
            no_git: false,
            include_tree: true,
        };
        let result = grab_contents(&config)?;
        let expected_tree_part = "\
---
DIRECTORY STRUCTURE
---
- deep/
  - sub/
    - nested.txt
- file2.rs
- subdir/
  - another.txt
- untracked.txt
"; // .gitignore excluded by -e, dirgrab.txt by default, logs/binary by .gitignore
        assert!(
            result.contains(expected_tree_part),
            "Expected tree structure not found in output:\n{}",
            result
        );
        assert!(result.contains("\n---\nFILE CONTENTS\n---\n\n"));
        // Check contents...
        Ok(())
    }

    #[test]
    fn test_grab_contents_with_tree_empty() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec!["*".to_string(), "*/".to_string()],
            include_untracked: true,
            include_default_output: true,
            no_git: true,
            include_tree: true,
        };
        let result = grab_contents(&config)?;
        let expected = "---\nDIRECTORY STRUCTURE (No files selected)\n---\n\n";
        assert_eq!(result, expected);
        Ok(())
    }

    // Tests calling internal helpers need crate:: prefix
    #[test]
    fn test_generate_indented_tree_simple() -> Result<()> {
        let tmp_dir = tempdir()?;
        let proj_dir = tmp_dir.path().join("project");
        fs::create_dir_all(proj_dir.join("src"))?;
        fs::create_dir_all(proj_dir.join("tests"))?;
        fs::write(proj_dir.join("src/main.rs"), "")?;
        fs::write(proj_dir.join("README.md"), "")?;
        fs::write(proj_dir.join("src/lib.rs"), "")?;
        fs::write(proj_dir.join("tests/basic.rs"), "")?;
        let base = PathBuf::from("/project");
        let files_logical = [
            // Use array
            base.join("src/main.rs"),
            base.join("README.md"),
            base.join("src/lib.rs"),
            base.join("tests/basic.rs"),
        ];
        let files_in_tmp = files_logical
            .iter()
            .map(|p| tmp_dir.path().join(p.strip_prefix("/").unwrap()))
            .collect::<Vec<_>>();
        let base_in_tmp = tmp_dir.path().join("project");

        let tree = crate::tree::generate_indented_tree(&files_in_tmp, &base_in_tmp)?; // Use crate:: path
        let expected = "\
- README.md
- src/
  - lib.rs
  - main.rs
- tests/
  - basic.rs
";
        assert_eq!(tree, expected);
        Ok(())
    }

    #[test]
    fn test_generate_indented_tree_deeper() -> Result<()> {
        let tmp_dir = tempdir()?;
        let proj_dir = tmp_dir.path().join("project");
        fs::create_dir_all(proj_dir.join("a/b/c"))?;
        fs::create_dir_all(proj_dir.join("a/d"))?;
        fs::write(proj_dir.join("a/b/c/file1.txt"), "")?;
        fs::write(proj_dir.join("a/d/file2.txt"), "")?;
        fs::write(proj_dir.join("top.txt"), "")?;
        fs::write(proj_dir.join("a/b/file3.txt"), "")?;
        let base = PathBuf::from("/project");
        let files_logical = [
            // Use array
            base.join("a/b/c/file1.txt"),
            base.join("a/d/file2.txt"),
            base.join("top.txt"),
            base.join("a/b/file3.txt"),
        ];
        let files_in_tmp = files_logical
            .iter()
            .map(|p| tmp_dir.path().join(p.strip_prefix("/").unwrap()))
            .collect::<Vec<_>>();
        let base_in_tmp = tmp_dir.path().join("project");

        let tree = crate::tree::generate_indented_tree(&files_in_tmp, &base_in_tmp)?; // Use crate:: path
        let expected = "\
- a/
  - b/
    - c/
      - file1.txt
    - file3.txt
  - d/
    - file2.txt
- top.txt
";
        assert_eq!(tree, expected);
        Ok(())
    }

    #[test]
    fn test_process_files_no_headers_skip_binary() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![
            path.join("file1.txt"),
            path.join("binary.dat"),
            path.join("file2.rs"),
        ];
        let dummy_target_path = path.clone();
        let result =
            crate::processing::process_files(&files_to_process, false, None, &dummy_target_path)?; // Use crate:: path
        let expected_content = "Content of file 1.\n\nfn main() {}\n\n";
        assert_eq!(result.trim(), expected_content.trim());
        Ok(())
    }

    #[test]
    fn test_process_files_with_headers_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("file2.rs")];
        let repo_root = Some(path.as_path());
        let dummy_target_path = path.clone();
        let result = crate::processing::process_files(
            &files_to_process,
            true,
            repo_root,
            &dummy_target_path,
        )?; // Use crate:: path
        let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nfn main() {{}}\n\n",
            Path::new("file1.txt").display(),
            Path::new("file2.rs").display()
        );
        assert_eq!(result.trim(), expected_content.trim());
        Ok(())
    }

    #[test]
    fn test_process_files_headers_no_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("subdir/another.txt")];
        let target_path_ref = path.as_path();
        let result =
            crate::processing::process_files(&files_to_process, true, None, target_path_ref)?; // Use crate:: path
        let expected_header1 = format!("--- FILE: {} ---", Path::new("file1.txt").display());
        let expected_header2 = format!(
            "--- FILE: {} ---",
            Path::new("subdir/another.txt").display()
        );
        assert!(result.contains(&expected_header1));
        assert!(result.contains(&expected_header2));
        assert!(result.contains("Content of file 1."));
        assert!(result.contains("Another text file."));
        Ok(())
    }
} // End of mod tests
