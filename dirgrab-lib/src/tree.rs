// --- FILE: dirgrab-lib/src/tree.rs ---

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use log::debug;

// Use crate:: errors because errors.rs is a sibling module declared in lib.rs
use crate::errors::{GrabError, GrabResult};

/// Generates an indented directory tree string from a list of file paths.
/// Crate-public as it's only called by grab_contents in lib.rs.
///
/// # Arguments
/// * `files`: A slice of absolute `PathBuf`s representing the files included
///   after all filtering.
/// * `base_path`: The absolute path (repo root or target path) relative to which
///   the tree structure should be displayed.
///
/// # Returns
/// * `Ok(String)` containing the formatted tree.
/// * `Err(GrabError::PathStripError)` if path relativization fails.
pub(crate) fn generate_indented_tree(files: &[PathBuf], base_path: &Path) -> GrabResult<String> {
    debug!("Generating tree relative to {:?}", base_path);
    let mut tree_output = String::new();
    // Use BTreeSet to store relative paths for automatic sorting and uniqueness.
    let mut relative_paths = BTreeSet::new();

    // Collect all unique relative paths (files and their parent directories)
    for file_path in files {
        // Strip the base_path to get the relative path for display
        let rel_path =
            file_path
                .strip_prefix(base_path)
                .map_err(|_e| GrabError::PathStripError {
                    // Use specific error
                    prefix: base_path.to_path_buf(),
                    path: file_path.clone(),
                })?;

        // Add the file itself
        relative_paths.insert(rel_path.to_path_buf());

        // Add all parent directories of the file
        let mut current = rel_path;
        while let Some(parent) = current.parent() {
            if parent.as_os_str().is_empty() {
                break; // Stop at the root "" path
            }
            relative_paths.insert(parent.to_path_buf());
            current = parent;
        }
    }

    // Build the indented string
    for rel_path in relative_paths {
        let depth = rel_path.components().count();
        let indent = "  ".repeat(depth.saturating_sub(1)); // Indent based on depth

        if let Some(name) = rel_path.file_name() {
            // Determine if it's a directory by checking its absolute path type
            // This requires the original base_path to reconstruct the absolute path.
            let abs_path = base_path.join(&rel_path);
            let is_dir = abs_path.is_dir(); // Relies on filesystem access

            tree_output.push_str(&format!(
                "{}- {}{}\n",
                indent,
                name.to_string_lossy(),
                if is_dir { "/" } else { "" }
            ));
        } else {
            // This case should generally not happen for file paths unless base_path itself is processed.
            // If base_path represents the root ".", we might see an empty path here.
            // For simplicity, we currently skip displaying the root explicitly in the tree.
            debug!(
                "Skipping empty path component in tree generation: {:?}",
                rel_path
            );
        }
    }

    Ok(tree_output)
}

// No specific tests for this module if covered by integration tests in lib.rs
// (Although unit testing generate_indented_tree could be beneficial later)
