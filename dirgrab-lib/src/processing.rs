// --- FILE: dirgrab-lib/src/processing.rs ---

use std::borrow::Cow;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use log::{debug, info, warn};

// Use crate:: paths for sibling modules
use crate::config::GrabConfig; // Import GrabConfig
use crate::errors::GrabResult;

#[derive(Debug, Clone)]
pub(crate) struct ProcessedFiles {
    pub content: String,
    pub files: Vec<FileSegment>,
}

#[derive(Debug, Clone)]
pub(crate) struct FileSegment {
    pub display_path: String,
    pub full_range: Range<usize>,
    pub header_range: Option<Range<usize>>,
    pub body_range: Range<usize>,
}

/// Reads a list of files, concatenates their UTF-8 content, optionally adding headers.
/// Handles PDF text extraction if configured.
/// Skips non-UTF8 files and files with read errors, logging warnings.
pub(crate) fn process_files(
    files: &[PathBuf],
    config: &GrabConfig,
    repo_root: Option<&Path>,
    target_path: &Path,
) -> GrabResult<ProcessedFiles> {
    debug!("Processing {} files for content.", files.len());
    let mut combined_content = String::with_capacity(files.len() * 1024);
    let mut segments = Vec::with_capacity(files.len());

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
        let display_path_ref = normalized_path(display_path);

        let file_start = combined_content.len();
        let mut header_range = None;
        let body_range;

        // --- Start PDF Handling ---
        let is_pdf = file_path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"));

        if config.convert_pdf && is_pdf {
            debug!("Attempting PDF text extraction for: {:?}", file_path);
            match pdf_extract::extract_text(file_path) {
                Ok(text) => {
                    if config.add_headers {
                        let header =
                            format!("--- FILE: {} (extracted text) ---\n", display_path_ref);
                        let start = combined_content.len();
                        combined_content.push_str(&header);
                        header_range = Some(start..combined_content.len());
                    }
                    let body_start = combined_content.len();
                    combined_content.push_str(&text);
                    if !text.ends_with('\n') {
                        combined_content.push('\n');
                    }
                    combined_content.push('\n');
                    body_range = body_start..combined_content.len();
                }
                Err(e) => {
                    warn!(
                        "Failed to extract text from PDF {:?}, skipping content: {}",
                        file_path, e
                    );
                    if config.add_headers {
                        let header = format!(
                            "--- FILE: {} (PDF extraction failed) ---\n",
                            display_path_ref
                        );
                        let start = combined_content.len();
                        combined_content.push_str(&header);
                        header_range = Some(start..combined_content.len());
                    }
                    let body_start = combined_content.len();
                    combined_content.push('\n');
                    body_range = body_start..combined_content.len();
                }
            }
        } else {
            // --- Regular File Handling (only if not handled as PDF) ---
            match fs::read(file_path) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(content) => {
                        if config.add_headers {
                            let header = format!("--- FILE: {} ---\n", display_path_ref);
                            let start = combined_content.len();
                            combined_content.push_str(&header);
                            header_range = Some(start..combined_content.len());
                        }
                        let body_start = combined_content.len();
                        combined_content.push_str(&content);
                        if !content.ends_with('\n') {
                            combined_content.push('\n');
                        }
                        combined_content.push('\n');
                        body_range = body_start..combined_content.len();
                    }
                    Err(_) => {
                        info!("Skipping non-UTF8 file: {:?}", file_path);
                        continue;
                    }
                },
                Err(e) => {
                    warn!("Skipping file due to read error: {:?} - {}", file_path, e);
                    continue;
                }
            }
        }

        let full_end = combined_content.len();
        segments.push(FileSegment {
            display_path: display_path_ref.to_string(),
            full_range: file_start..full_end,
            header_range,
            body_range,
        });
    } // End of loop through files

    Ok(ProcessedFiles {
        content: combined_content,
        files: segments,
    })
}

fn normalized_path(path: &Path) -> Cow<'_, str> {
    let raw = path.to_string_lossy();
    if std::path::MAIN_SEPARATOR == '\\' && raw.contains('\\') {
        Cow::Owned(raw.replace('\\', "/"))
    } else {
        raw
    }
}
