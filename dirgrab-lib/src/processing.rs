// --- FILE: dirgrab-lib/src/processing.rs ---

use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use log::{debug, warn};

// Use crate:: paths for sibling modules
use crate::config::GrabConfig; // Import GrabConfig
use crate::errors::GrabResult;

/// Reads a list of files, concatenates their UTF-8 content, optionally adding headers.
/// Handles PDF text extraction if configured.
/// Skips non-UTF8 files and files with read errors, logging warnings.
pub(crate) fn process_files(
    files: &[PathBuf],
    config: &GrabConfig,
    repo_root: Option<&Path>,
    target_path: &Path,
) -> GrabResult<String> {
    debug!("Processing {} files for content.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024);
    let mut buffer = Vec::new(); // Reusable buffer for reading

    for file_path in files {
        debug!("Processing file content for: {:?}", file_path);

        let display_path_result = if !config.no_git {
            // Simplified condition - repo_root being Some is implied by if let
            if let Some(repo_root_ref) = repo_root {
                // Renamed to repo_root_ref for clarity
                file_path.strip_prefix(repo_root_ref) // Use repo_root_ref here
            } else {
                file_path.strip_prefix(target_path) // Fallback if repo_root is None (though unlikely in Git mode)
            }
        } else {
            file_path.strip_prefix(target_path) // Non-Git mode always strips from target_path
        };
        let display_path = display_path_result.unwrap_or(file_path);

        // --- Start PDF Handling ---
        let is_pdf = file_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));

        if config.convert_pdf && is_pdf {
            debug!("Attempting PDF text extraction for: {:?}", file_path);
            match pdf_extract::extract_text(file_path) {
                Ok(text) => {
                    if config.add_headers {
                        combined_content.push_str(&format!(
                            "--- FILE: {} (extracted text) ---\n",
                            display_path.display()
                        ));
                    }
                    combined_content.push_str(&text);
                    if !text.ends_with('\n') {
                        combined_content.push('\n');
                    }
                    combined_content.push('\n');
                    continue;
                }
                Err(e) => {
                    warn!(
                        "Failed to extract text from PDF {:?}, skipping content: {}",
                        file_path, e
                    );
                    if config.add_headers {
                        combined_content.push_str(&format!(
                            "--- FILE: {} (PDF extraction failed) ---\n\n",
                            display_path.display()
                        ));
                    }
                    continue;
                }
            }
        }
        // --- End PDF Handling ---

        // --- Regular File Handling (only if not handled as PDF) ---

        // --- Read File Content (Header addition moved below) ---
        buffer.clear();
        match fs::File::open(file_path) {
            Ok(file) => {
                let mut reader = BufReader::new(file);
                match reader.read_to_end(&mut buffer) {
                    Ok(_) => {
                        match String::from_utf8(buffer.clone()) {
                            Ok(content) => {
                                // *** Add header ONLY on successful UTF-8 decode ***
                                if config.add_headers {
                                    combined_content.push_str(&format!(
                                        "--- FILE: {} ---\n", // Regular header
                                        display_path.display()
                                    ));
                                }
                                combined_content.push_str(&content);
                                if !content.ends_with('\n') {
                                    combined_content.push('\n');
                                }
                                combined_content.push('\n'); // Extra newline
                            }
                            Err(_) => {
                                warn!("Skipping non-UTF8 file: {:?}", file_path);
                                // No header is added if UTF-8 decoding fails
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Skipping file due to read error: {:?} - {}", file_path, e);
                        // No header is added if read fails
                    }
                }
            }
            Err(e) => {
                warn!("Skipping file due to open error: {:?} - {}", file_path, e);
                // No header is added if open fails
            }
        }
        // --- End Regular File Handling ---
    } // End of loop through files

    Ok(combined_content)
}
