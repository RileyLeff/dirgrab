#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))]

// Declare modules
mod config;
mod errors;
mod listing;
mod processing;
mod tree;
mod utils;

// Necessary imports for lib.rs itself
use log::{debug, error, info, warn};
use std::io; // For io::ErrorKind // For logging within grab_contents
use std::ops::Range;
use std::path::{Path, PathBuf};

// Re-export public API components
pub use config::GrabConfig;
pub use errors::{GrabError, GrabResult};

#[derive(Debug, Clone)]
pub struct GrabbedFile {
    pub display_path: String,
    pub full_range: Range<usize>,
    pub header_range: Option<Range<usize>>,
    pub body_range: Range<usize>,
}

#[derive(Debug, Clone)]
pub struct GrabOutput {
    pub content: String,
    pub files: Vec<GrabbedFile>,
}

// --- Main Public Function ---

/// Performs the main `dirgrab` operation based on the provided configuration.
pub fn grab_contents(config: &GrabConfig) -> GrabResult<String> {
    grab_contents_detailed(config).map(|output| output.content)
}

/// Performs the main `dirgrab` operation and returns file-level metadata along with the content.
pub fn grab_contents_detailed(config: &GrabConfig) -> GrabResult<GrabOutput> {
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
        info!("Ignoring Git context due to --no-git flag.");
        let files = listing::list_files_walkdir(&target_path, config)?;
        (files, None)
    } else {
        let git_repo_root = listing::detect_git_repo(&target_path)?;
        let scope_subdir = git_repo_root
            .as_ref()
            .and_then(|root| derive_scope_subdir(root, &target_path, config));

        let files = match &git_repo_root {
            Some(root) => {
                info!("Operating in Git mode. Repo root: {:?}", root);
                if let Some(scope) = scope_subdir.as_deref() {
                    info!("Limiting Git file listing to sub-path: {:?}", scope);
                } else if !config.all_repo {
                    debug!(
                        "Scope calculation yielded full repository; processing entire repo contents."
                    );
                }
                listing::list_files_git(root, config, scope_subdir.as_deref())?
            }
            None => {
                info!("Operating in Non-Git mode. Target path: {:?}", target_path);
                listing::list_files_walkdir(&target_path, config)?
            }
        };
        (files, git_repo_root)
    };

    info!("Found {} files to process.", files_to_process.len());

    // Initialize output buffer
    let mut output_buffer = String::new();
    let mut file_segments = Vec::new();

    // Generate and prepend tree if requested
    if config.include_tree {
        if files_to_process.is_empty() {
            warn!("--include-tree specified, but no files were selected for processing. Tree will be empty.");
            // Keep explicit tree header even if empty
            output_buffer.push_str("---\nDIRECTORY STRUCTURE (No files selected)\n---\n\n");
            return Ok(GrabOutput {
                content: output_buffer,
                files: Vec::new(),
            });
        } else {
            // Determine base path for tree (repo root if git mode, target path otherwise)
            let base_path_for_tree = if !config.no_git && maybe_repo_root.is_some() {
                maybe_repo_root.as_deref().unwrap() // Safe unwrap due to is_some() check
            } else {
                &target_path
            };
            debug!(
                "Generating directory tree relative to: {:?}",
                base_path_for_tree
            );

            match tree::generate_indented_tree(&files_to_process, base_path_for_tree) {
                Ok(tree_str) => {
                    output_buffer.push_str("---\nDIRECTORY STRUCTURE\n---\n");
                    output_buffer.push_str(&tree_str);
                    output_buffer.push_str("\n---\nFILE CONTENTS\n---\n\n");
                }
                Err(e) => {
                    error!("Failed to generate directory tree: {}", e);
                    // Still add header indicating failure
                    output_buffer.push_str("---\nERROR GENERATING DIRECTORY STRUCTURE\n---\n\n");
                }
            }
        }
    }

    // Process files and append content (only if files exist)
    if !files_to_process.is_empty() {
        // Updated call to process_files to pass the whole config struct
        let processed = processing::process_files(
            &files_to_process,
            config, // Pass config struct
            maybe_repo_root.as_deref(),
            &target_path,
        )?;
        let base_offset = output_buffer.len();
        output_buffer.push_str(&processed.content);
        for segment in processed.files {
            file_segments.push(GrabbedFile {
                display_path: segment.display_path,
                full_range: offset_range(&segment.full_range, base_offset),
                header_range: segment
                    .header_range
                    .map(|range| offset_range(&range, base_offset)),
                body_range: offset_range(&segment.body_range, base_offset),
            });
        }
    } else if !config.include_tree {
        // If no files AND no tree was requested
        warn!("No files selected for processing based on current configuration.");
        // Return empty string only if no files were found AND tree wasn't requested/generated.
        return Ok(GrabOutput {
            content: String::new(),
            files: Vec::new(),
        });
    }

    // Return the combined buffer (might contain only tree, or tree + content, or just content)
    Ok(GrabOutput {
        content: output_buffer,
        files: file_segments,
    })
}

fn derive_scope_subdir(
    repo_root: &Path,
    target_path: &Path,
    config: &GrabConfig,
) -> Option<PathBuf> {
    if config.all_repo {
        return None;
    }

    match target_path.strip_prefix(repo_root) {
        Ok(rel) => {
            if rel.as_os_str().is_empty() {
                None
            } else {
                Some(rel.to_path_buf())
            }
        }
        Err(_) => None,
    }
}

fn offset_range(range: &Range<usize>, offset: usize) -> Range<usize> {
    (range.start + offset)..(range.end + offset)
}

// --- FILE: dirgrab-lib/src/lib.rs ---
// (Showing only the tests module and its necessary imports)

// ... (rest of lib.rs code above) ...

// --- Tests ---
#[cfg(test)]
mod tests {
    // Use super::* to bring everything from lib.rs into scope for tests
    // This now includes GrabConfig, GrabError, GrabResult because they are re-exported.
    use super::*;
    // Also need direct imports for helpers/types used *only* in tests
    use anyhow::{Context, Result}; // Ensure Context and Result are imported from anyhow
    use std::collections::HashSet;
    use std::fs::{self}; // Ensure File is imported if needed by helpers
    use std::path::{Path, PathBuf}; // Need these for helpers defined within tests mod
    use std::process::Command;
    use tempfile::{tempdir, TempDir};

    // --- Test Setup Helpers ---
    fn setup_test_dir() -> Result<(TempDir, PathBuf)> {
        let dir = tempdir()?;
        let path = dir.path().to_path_buf();

        fs::write(path.join("file1.txt"), "Content of file 1.")?;
        fs::write(path.join("file2.rs"), "fn main() {}")?;
        fs::create_dir_all(path.join("subdir"))?; // Use create_dir_all
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
        // Configure Git to handle potential CRLF issues on Windows in tests if needed
        crate::utils::run_command("git", &["config", "core.autocrlf", "false"], path)?;

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

    fn run_test_command(
        cmd: &str,
        args: &[&str],
        current_dir: &Path,
    ) -> Result<std::process::Output> {
        let output = crate::utils::run_command(cmd, args, current_dir)?;
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
        assert_eq!(
            actual_set, expected,
            "Path sets differ.\nActual paths: {:?}\nExpected paths: {:?}",
            actual_set, expected
        );
    }

    // --- Tests ---
    // Tests calling listing functions need crate:: prefix
    #[test]
    fn test_detect_git_repo_inside() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
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
        // Ensure no git repo exists here
        let maybe_root = crate::listing::detect_git_repo(&path)?; // Use crate:: path
        assert!(maybe_root.is_none());
        Ok(())
    }

    #[test]
    fn test_list_files_walkdir_no_exclude_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,      // No effect in walkdir
            include_default_output: false, // Exclude dirgrab.txt
            no_git: true,                  // Force walkdir
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
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
                // "dirgrab.txt" should be excluded by default
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
            exclude_patterns: vec!["*.log".to_string(), "subdir/".to_string()], // User excludes
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Force walkdir
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_walkdir(&path, &config)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                "file1.txt",
                "file2.rs",
                "binary.dat",
                // subdir/* excluded
                // dirgrab.txt excluded by default
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_tracked_only_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(), // Target doesn't matter as much as root for list_files_git
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,      // Tracked only
            include_default_output: false, // Exclude dirgrab.txt
            no_git: false,                 // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path, pass repo root
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                // file1.txt ignored by .gitignore
                // file3.log ignored by .gitignore
                // binary.dat ignored by .gitignore
                // dirgrab.txt not tracked and default excluded
                // untracked.txt not tracked
                // ignored.log not tracked
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_untracked_default_excludes_dirgrab_txt() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,       // Include untracked
            include_default_output: false, // Exclude dirgrab.txt
            no_git: false,                 // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt", // Included now
                                 // file1.txt ignored by .gitignore
                                 // file3.log ignored by .gitignore
                                 // binary.dat ignored by .gitignore
                                 // ignored.log ignored by .gitignore (via --exclude-standard)
                                 // dirgrab.txt untracked and default excluded
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![
                "*.rs".to_string(),    // Exclude rust files
                "subdir/".to_string(), // Exclude subdir/
                "deep/".to_string(),   // Exclude deep/
            ],
            include_untracked: false, // Tracked only
            include_default_output: false,
            no_git: false, // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(&path, &[".gitignore"]); // Only .gitignore remains
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_untracked_with_exclude() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["*.txt".to_string()], // Exclude all .txt files
            include_untracked: true,                     // Include untracked
            include_default_output: false,
            no_git: false, // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                // subdir/another.txt excluded by *.txt
                // deep/sub/nested.txt excluded by *.txt
                // untracked.txt excluded by *.txt
                // dirgrab.txt excluded by default
            ],
        );
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
            include_default_output: true, // Include dirgrab.txt
            no_git: true,                 // Force walkdir
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
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
                "dirgrab.txt", // Included now
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_tracked_only() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        // Make dirgrab.txt tracked
        fs::write(path.join("dirgrab.txt"), "Tracked dirgrab output.")?;
        run_test_command("git", &["add", "dirgrab.txt"], &path)?;
        run_test_command("git", &["commit", "-m", "Add dirgrab.txt"], &path)?;

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,     // Tracked only
            include_default_output: true, // Include dirgrab.txt
            no_git: false,                // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "dirgrab.txt", // Included because tracked and override flag set
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_with_untracked() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        // dirgrab.txt is untracked in this setup
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,      // Include untracked
            include_default_output: true, // Include dirgrab.txt
            no_git: false,                // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt", // Included
                "dirgrab.txt",   // Included because untracked and override flag set
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_include_default_output_but_excluded_by_user() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["dirgrab.txt".to_string()], // User explicitly excludes
            include_untracked: true,
            include_default_output: true, // Override default exclusion, but user exclusion takes precedence
            no_git: false,                // Use Git
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let files = crate::listing::list_files_git(&path, &config, None)?; // Use crate:: path
        let expected_set = get_expected_set(
            &path,
            &[
                ".gitignore",
                "file2.rs",
                "subdir/another.txt",
                "deep/sub/nested.txt",
                "untracked.txt",
                // dirgrab.txt excluded by user pattern
            ],
        );
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_list_files_git_scoped_to_subdir() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }

        fs::write(path.join("deep/untracked_inside.txt"), "scoped content")?;

        let config = GrabConfig {
            target_path: path.join("deep"),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: true,
            include_default_output: false,
            no_git: false,
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let scope = Path::new("deep");
        let files = crate::listing::list_files_git(&path, &config, Some(scope))?;
        let expected_set =
            get_expected_set(&path, &["deep/sub/nested.txt", "deep/untracked_inside.txt"]);
        assert_paths_eq(files, expected_set);
        Ok(())
    }

    #[test]
    fn test_no_git_flag_forces_walkdir_in_git_repo() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false, // No headers for easier content check
            exclude_patterns: vec![],
            include_untracked: false,      // No effect
            include_default_output: false, // Exclude dirgrab.txt
            no_git: true,                  // Force walkdir
            include_tree: false,           // No tree for easier content check
            convert_pdf: false,
            all_repo: false,
        };
        let result_string = grab_contents(&config)?;

        // Check content from files that would be ignored by git but included by walkdir
        assert!(
            result_string.contains("Content of file 1."),
            "file1.txt content missing"
        ); // Ignored by .gitignore, but walkdir includes
        assert!(
            result_string.contains("Log message."),
            "file3.log content missing"
        ); // Ignored by .gitignore, but walkdir includes
        assert!(
            result_string.contains("fn main() {}"),
            "file2.rs content missing"
        ); // Tracked by git, included by walkdir
        assert!(
            result_string.contains("Another text file."),
            "another.txt content missing"
        ); // Tracked by git, included by walkdir
        assert!(
            !result_string.contains("Previous dirgrab output."),
            "dirgrab.txt included unexpectedly"
        ); // Excluded by default

        // The binary file binary.dat is skipped because it's not valid UTF-8.
        // The processing function logs a warning. We don't need to assert its absence
        // in the final string, as it cannot be represented in a valid Rust String anyway.
        // The fact that grab_contents completes successfully and includes the text files is sufficient.

        Ok(())
    }

    #[test]
    fn test_no_git_flag_still_respects_exclude_patterns() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec!["*.txt".to_string(), "*.rs".to_string()], // Exclude .txt and .rs
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Force walkdir
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let result_string = grab_contents(&config)?;

        assert!(result_string.contains("Log message."), "file3.log missing"); // Included
        assert!(
            !result_string.contains("Content of file 1."),
            "file1.txt included unexpectedly"
        ); // Excluded by *.txt
        assert!(
            !result_string.contains("fn main() {}"),
            "file2.rs included unexpectedly"
        ); // Excluded by *.rs
        assert!(
            !result_string.contains("Another text file."),
            "another.txt included unexpectedly"
        ); // Excluded by *.txt
        assert!(
            !result_string.contains("Nested content"),
            "nested.txt included unexpectedly"
        ); // Excluded by *.txt
        assert!(
            !result_string.contains("Previous dirgrab output."),
            "dirgrab.txt included unexpectedly"
        ); // Excluded by default & *.txt

        Ok(())
    }

    #[test]
    fn test_no_git_flag_with_include_default_output() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: false,
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: true, // Include dirgrab.txt
            no_git: true,                 // Force walkdir
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let result_string = grab_contents(&config)?;
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
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(), // Target is repo root
            add_headers: true,         // Enable headers
            exclude_patterns: vec![
                "*.log".to_string(),
                "*.dat".to_string(),
                "dirgrab.txt".to_string(),
            ], // Simplify output
            include_untracked: false,
            include_default_output: false,
            no_git: true,        // Force walkdir
            include_tree: false, // No tree
            convert_pdf: false,
            all_repo: false,
        };
        let result_string = grab_contents(&config)?;

        // file1.txt is ignored by .gitignore but included here because no_git=true
        let expected_header_f1 = format!("--- FILE: {} ---", Path::new("file1.txt").display());
        assert!(
            result_string.contains(&expected_header_f1),
            "Header path should be relative to target_path. Expected '{}' in output:\n{}",
            expected_header_f1,
            result_string
        );

        // .gitignore itself is not usually listed by walkdir unless explicitly targeted? Let's check file2.rs
        let expected_header_f2 = format!("--- FILE: {} ---", Path::new("file2.rs").display());
        assert!(
            result_string.contains(&expected_header_f2),
            "Header path should be relative to target_path. Expected '{}' in output:\n{}",
            expected_header_f2,
            result_string
        );

        let expected_nested_header = format!(
            "--- FILE: {} ---",
            Path::new("deep/sub/nested.txt").display()
        );
        assert!(
            result_string.contains(&expected_nested_header),
            "Nested header path relative to target_path. Expected '{}' in output:\n{}",
            expected_nested_header,
            result_string
        );
        Ok(())
    }

    #[test]
    fn test_git_mode_headers_relative_to_repo_root() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let subdir_target = path.join("deep"); // Target is inside the repo
        fs::create_dir_all(&subdir_target)?; // Ensure target exists

        let config = GrabConfig {
            target_path: subdir_target.clone(), // Target is 'deep' subdir
            add_headers: true,                  // Enable headers
            exclude_patterns: vec![],
            include_untracked: false, // Tracked only
            include_default_output: false,
            no_git: false,       // Use Git mode
            include_tree: false, // No tree
            convert_pdf: false,
            all_repo: false,
        };
        let result_string = grab_contents(&config)?; // Should still find files relative to repo root

        // Check headers are relative to repo root (path), not target_path (subdir_target)
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

        // Check other files outside the target dir are also included and relative to root
        let unexpected_root_header = format!("--- FILE: {} ---", Path::new(".gitignore").display());
        assert!(
            !result_string.contains(&unexpected_root_header),
            "Scoped results should not include repo-root files. Unexpected '{}' in output:\n{}",
            unexpected_root_header,
            result_string
        );
        let unexpected_rs_header = format!("--- FILE: {} ---", Path::new("file2.rs").display());
        assert!(
            !result_string.contains(&unexpected_rs_header),
            "Scoped results should not include repo-root files. Unexpected '{}' in output:\n{}",
            unexpected_rs_header,
            result_string
        );
        Ok(())
    }

    #[test]
    fn test_grab_contents_with_tree_no_git() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        // Don't need git repo setup for no_git test, but keep files consistent
        fs::write(path.join(".gitignore"), "*.log\nbinary.dat")?; // Create dummy .gitignore
        fs::create_dir_all(path.join("deep/sub"))?;
        fs::write(path.join("deep/sub/nested.txt"), "Nested content")?;
        fs::write(path.join("untracked.txt"), "Untracked content")?; // File exists

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![
                "*.log".to_string(),       // Exclude logs
                "*.dat".to_string(),       // Exclude binary
                ".gitignore".to_string(),  // Exclude .gitignore itself
                "dirgrab.txt".to_string(), // Exclude default output file explicitly too
            ],
            include_untracked: false,      // No effect
            include_default_output: false, // Also excluded above
            no_git: true,                  // Force walkdir
            include_tree: true,            // THE flag to test
            convert_pdf: false,
            all_repo: false,
        };
        let result = grab_contents(&config)?;

        // Expected tree for walkdir with excludes applied
        // file1.txt, file2.rs, another.txt, nested.txt, untracked.txt should remain
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
";

        assert!(
            result.contains(expected_tree_part),
            "Expected tree structure not found in output:\nTree Section:\n---\n{}\n---",
            result
                .split("---\nFILE CONTENTS\n---")
                .next()
                .unwrap_or("TREE NOT FOUND")
        );

        assert!(
            result.contains("\n---\nFILE CONTENTS\n---\n\n"),
            "Expected file content separator not found"
        );
        // Check presence of headers and content for included files
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
        // Check absence of excluded file content
        assert!(
            !result.contains("Previous dirgrab output."),
            "dirgrab.txt content included unexpectedly"
        );
        assert!(
            !result.contains("Log message"),
            "Log content included unexpectedly"
        );

        Ok(())
    }

    #[test]
    fn test_grab_contents_with_tree_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        if !setup_git_repo(&path)? {
            println!("Skipping Git test: git not found or setup failed.");
            return Ok(());
        }
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![".gitignore".to_string()], // Exclude .gitignore
            include_untracked: true,                          // Include untracked
            include_default_output: false,                    // Exclude dirgrab.txt (default)
            no_git: false,                                    // Use Git
            include_tree: true,                               // Include tree
            convert_pdf: false,
            all_repo: false,
        };
        let result = grab_contents(&config)?;

        // Expected tree for git ls-files -ou --exclude-standard :!.gitignore :!dirgrab.txt
        // Should include: file2.rs, another.txt, nested.txt, untracked.txt
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
";
        assert!(
            result.contains(expected_tree_part),
            "Expected tree structure not found in output:\nTree Section:\n---\n{}\n---",
            result
                .split("---\nFILE CONTENTS\n---")
                .next()
                .unwrap_or("TREE NOT FOUND")
        );
        assert!(
            result.contains("\n---\nFILE CONTENTS\n---\n\n"),
            "Separator missing"
        );
        // Check content
        assert!(
            result.contains("--- FILE: file2.rs ---"),
            "file2.rs header missing"
        );
        assert!(result.contains("fn main() {}"), "file2.rs content missing");
        assert!(
            result.contains("--- FILE: untracked.txt ---"),
            "untracked.txt header missing"
        );
        assert!(
            result.contains("This file is not tracked."),
            "untracked.txt content missing"
        );
        assert!(
            !result.contains("--- FILE: .gitignore ---"),
            ".gitignore included unexpectedly"
        );

        Ok(())
    }

    #[test]
    fn test_grab_contents_with_tree_empty() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        // No need for files if we exclude everything
        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec!["*".to_string(), "*/".to_string()], // Exclude everything
            include_untracked: true,
            include_default_output: true,
            no_git: true,       // Use walkdir
            include_tree: true, // Ask for tree
            convert_pdf: false,
            all_repo: false,
        };
        let result = grab_contents(&config)?;
        // Expect only the empty tree message
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

        // Simulate paths relative to a base (doesn't have to exist for this test)
        let base = PathBuf::from("/project"); // Logical base
        let files_logical = [
            // Use array for BTreeSet later if needed
            base.join("src/main.rs"),
            base.join("README.md"),
            base.join("src/lib.rs"),
            base.join("tests/basic.rs"),
        ];

        // Map logical paths to actual paths in temp dir for is_dir() check
        let files_in_tmp = files_logical
            .iter()
            .map(|p| tmp_dir.path().join(p.strip_prefix("/").unwrap()))
            .collect::<Vec<_>>();
        let base_in_tmp = tmp_dir.path().join("project"); // The actual base path

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

        let base = PathBuf::from("/project"); // Logical base
        let files_logical = [
            base.join("a/b/c/file1.txt"),
            base.join("a/d/file2.txt"),
            base.join("top.txt"),
            base.join("a/b/file3.txt"),
        ];

        let files_in_tmp = files_logical
            .iter()
            .map(|p| tmp_dir.path().join(p.strip_prefix("/").unwrap()))
            .collect::<Vec<_>>();
        let base_in_tmp = tmp_dir.path().join("project"); // Actual base

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

    // --- Tests for processing.rs (Updated to pass GrabConfig) ---
    #[test]
    fn test_process_files_no_headers_skip_binary() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![
            path.join("file1.txt"),
            path.join("binary.dat"), // Should be skipped as non-utf8
            path.join("file2.rs"),
        ];
        let config = GrabConfig {
            // Create dummy config
            target_path: path.clone(),
            add_headers: false, // Key part of this test
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Assume non-git mode for simplicity here
            include_tree: false,
            convert_pdf: false, // PDF conversion off
            all_repo: false,
        };
        let result = crate::processing::process_files(&files_to_process, &config, None, &path)?;
        let expected_content = "Content of file 1.\n\nfn main() {}\n\n";
        assert_eq!(result.content, expected_content);
        assert_eq!(result.files.len(), 2);
        assert_eq!(result.files[0].display_path, "file1.txt");
        assert!(result.files[0].header_range.is_none());
        assert_eq!(
            &result.content[result.files[0].body_range.clone()],
            "Content of file 1.\n\n"
        );
        assert_eq!(
            &result.content[result.files[1].body_range.clone()],
            "fn main() {}\n\n"
        );
        Ok(())
    }

    #[test]
    fn test_process_files_with_headers_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        // Don't need full git setup if we just provide repo_root
        let files_to_process = vec![path.join("file1.txt"), path.join("file2.rs")];
        let repo_root = Some(path.as_path());
        let config = GrabConfig {
            target_path: path.clone(), // target can be same as root for this test
            add_headers: true,         // Key part of this test
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: false, // Git mode ON
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let result =
            crate::processing::process_files(&files_to_process, &config, repo_root, &path)?;
        let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nfn main() {{}}\n\n",
            Path::new("file1.txt").display(), // Paths relative to repo_root (which is path)
            Path::new("file2.rs").display()
        );
        assert_eq!(result.content, expected_content);
        assert_eq!(result.files.len(), 2);
        assert!(result.files.iter().all(|seg| seg.header_range.is_some()));
        let first = &result.files[0];
        assert_eq!(first.display_path, "file1.txt");
        assert_eq!(
            &result.content[first.header_range.clone().unwrap()],
            "--- FILE: file1.txt ---\n"
        );
        assert_eq!(
            &result.content[first.body_range.clone()],
            "Content of file 1.\n\n"
        );
        Ok(())
    }

    #[test]
    fn test_process_files_headers_no_git_mode() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let files_to_process = vec![path.join("file1.txt"), path.join("subdir/another.txt")];
        let config = GrabConfig {
            target_path: path.clone(), // Target path is the base
            add_headers: true,         // Key part of this test
            exclude_patterns: vec![],
            include_untracked: false,
            include_default_output: false,
            no_git: true, // Git mode OFF
            include_tree: false,
            convert_pdf: false,
            all_repo: false,
        };
        let result = crate::processing::process_files(&files_to_process, &config, None, &path)?;
        let expected_content = format!(
            "--- FILE: {} ---\nContent of file 1.\n\n--- FILE: {} ---\nAnother text file.\n\n",
            Path::new("file1.txt").display(), // Paths relative to target_path
            Path::new("subdir/another.txt").display()
        );
        assert_eq!(result.content, expected_content);
        assert_eq!(result.files.len(), 2);
        Ok(())
    }

    #[test]
    fn test_grab_contents_with_pdf_conversion_enabled() -> Result<()> {
        let (_dir, path) = setup_test_dir()?;
        let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixtures_dir = base_dir.join("tests/fixtures");
        fs::create_dir_all(&fixtures_dir)?;
        let fixture_pdf_src = fixtures_dir.join("sample.pdf");

        if !fixture_pdf_src.exists() {
            anyhow::bail!("Fixture PDF not found at {:?}", fixture_pdf_src);
        }

        let fixture_pdf_dest = path.join("sample.pdf");
        fs::copy(&fixture_pdf_src, &fixture_pdf_dest).with_context(|| {
            format!(
                "Failed to copy fixture PDF from {:?} to {:?}",
                fixture_pdf_src, fixture_pdf_dest
            )
        })?;

        fs::write(path.join("normal.txt"), "Normal text content.")?;

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            exclude_patterns: vec![
                "dirgrab.txt".into(),
                "*.log".into(),
                "*.dat".into(),
                "*.rs".into(),
                "subdir/".into(),
                ".gitignore".into(),
                "deep/".into(),
                "untracked.txt".into(),
            ],
            include_untracked: false,
            include_default_output: false,
            no_git: true,
            include_tree: false,
            convert_pdf: true,
            all_repo: false,
        };

        let result_string = grab_contents(&config)?;

        // Check PDF header
        let expected_pdf_header = "--- FILE: sample.pdf (extracted text) ---";
        assert!(
            result_string.contains(expected_pdf_header),
            "Missing or incorrect PDF header. Output:\n{}",
            result_string
        );

        // *** Update expected content based on actual PDF text - try a different snippet ***
        // let expected_pdf_content = "Pines are the largest and most"; // Original snippet
        let expected_pdf_content = "Pinaceae family"; // Try this snippet instead

        // Add a println to see exactly what is being searched for and in what
        println!("Searching for: '{}'", expected_pdf_content);
        println!("Within: '{}'", result_string);

        assert!(
            result_string.contains(expected_pdf_content),
            "Missing extracted PDF content ('{}'). Output:\n{}",
            expected_pdf_content,
            result_string
        );

        // Check normal text file header and content
        let expected_txt_header = "--- FILE: normal.txt ---";
        let expected_txt_content = "Normal text content.";
        assert!(
            result_string.contains(expected_txt_header),
            "Missing or incorrect TXT header. Output:\n{}",
            result_string
        );
        assert!(
            result_string.contains(expected_txt_content),
            "Missing TXT content. Output:\n{}",
            result_string
        );

        // Check that file1.txt (not excluded) is present
        let expected_file1_header = "--- FILE: file1.txt ---";
        assert!(
            result_string.contains(expected_file1_header),
            "Missing file1.txt header. Output:\n{}",
            result_string
        );

        Ok(())
    }

    #[test]
    fn test_grab_contents_with_pdf_conversion_disabled() -> Result<()> {
        let (_dir, path) = setup_test_dir()?; // Use existing helper
        let base_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixtures_dir = base_dir.join("tests/fixtures");
        fs::create_dir_all(&fixtures_dir)?; // Ensure exists
        let fixture_pdf_src = fixtures_dir.join("sample.pdf");

        // Create dummy if needed
        if !fixture_pdf_src.exists() {
            let basic_pdf_content = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Count 1/Kids[3 0 R]>>endobj\n3 0 obj<</Type/Page/MediaBox[0 0 612 792]/Contents 4 0 R/Resources<<>>>>endobj\n4 0 obj<</Length 52>>stream\nBT /F1 12 Tf 72 712 Td (This is sample PDF text content.) Tj ET\nendstream\nendobj\nxref\n0 5\n0000000000 65535 f \n0000000010 00000 n \n0000000063 00000 n \n0000000117 00000 n \n0000000198 00000 n \ntrailer<</Size 5/Root 1 0 R>>\nstartxref\n315\n%%EOF";
            fs::write(&fixture_pdf_src, basic_pdf_content)?;
            println!(
                "Created dummy sample.pdf for testing at {:?}",
                fixture_pdf_src
            );
        }

        let fixture_pdf_dest = path.join("sample.pdf");
        fs::copy(&fixture_pdf_src, &fixture_pdf_dest).with_context(|| {
            format!(
                "Failed to copy fixture PDF from {:?} to {:?}",
                fixture_pdf_src, fixture_pdf_dest
            )
        })?;
        fs::write(path.join("normal.txt"), "Normal text content.")?;

        let config = GrabConfig {
            target_path: path.clone(),
            add_headers: true,
            // Exclude many things to simplify output check
            exclude_patterns: vec![
                "dirgrab.txt".into(),
                "*.log".into(),
                "*.dat".into(),
                "*.rs".into(),
                "subdir/".into(),
                ".gitignore".into(),
                "deep/".into(),
                "untracked.txt".into(),
            ],
            include_untracked: false,
            include_default_output: false,
            no_git: true,
            include_tree: false,
            convert_pdf: false, // Disable PDF conversion
            all_repo: false,
        };

        let result_string = grab_contents(&config)?;

        // Check PDF is NOT processed as text
        let unexpected_pdf_header_part = "(extracted text)"; // Check for the specific part of the header
        let unexpected_pdf_content = "This is sample PDF text content.";
        assert!(
            !result_string.contains(unexpected_pdf_header_part),
            "PDF extracted text header part present unexpectedly. Output:\n{}",
            result_string
        );
        assert!(
            !result_string.contains(unexpected_pdf_content),
            "Extracted PDF content present unexpectedly. Output:\n{}",
            result_string
        );

        // Check normal text file is still included
        let expected_txt_header = "--- FILE: normal.txt ---";
        let expected_txt_content = "Normal text content.";
        assert!(
            result_string.contains(expected_txt_header),
            "Missing or incorrect TXT header. Output:\n{}",
            result_string
        );
        assert!(
            result_string.contains(expected_txt_content),
            "Missing TXT content. Output:\n{}",
            result_string
        );

        // Check that file1.txt (not excluded) is present
        let expected_file1_header = "--- FILE: file1.txt ---";
        assert!(
            result_string.contains(expected_file1_header),
            "Missing file1.txt header. Output:\n{}",
            result_string
        );

        // With convert_pdf: false, the PDF should be skipped as non-UTF8 by the fallback logic.
        // Check that the standard PDF header does NOT appear either.
        let regular_pdf_header = "--- FILE: sample.pdf ---";
        assert!(
            !result_string.contains(regular_pdf_header),
            "Regular PDF header present when it should have been skipped as non-utf8. Output:\n{}",
            result_string
        );

        Ok(())
    }
} // End of mod tests
