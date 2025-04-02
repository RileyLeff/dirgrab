// --- FILE: dirgrab-lib/src/processing.rs ---

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use log::{debug, warn};

// Use crate:: errors because errors.rs is a sibling module declared in lib.rs
use crate::errors::GrabResult; // Only need the Result type alias here

/// Reads a list of files, concatenates their UTF-8 content, optionally adding headers.
/// Skips non-UTF8 files and files with read errors, logging warnings.
/// Crate-public as it's only called by grab_contents in lib.rs.
pub(crate) fn process_files(
    files: &[PathBuf],
    add_headers: bool,
    repo_root: Option<&Path>,
    target_path: &Path, // Needed for non-git relative paths in headers
) -> GrabResult<String> {
    debug!("Processing {} files for content.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024);
    let mut buffer = Vec::new(); // Reusable buffer for reading

    for file_path in files {
        debug!("Processing file content for: {:?}", file_path);

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

// No specific tests for this module if covered by integration tests in lib.rs
