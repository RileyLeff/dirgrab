// --- FILE: dirgrab-lib/src/utils.rs ---

use std::io; // Needed for io::ErrorKind::NotFound check
use std::path::Path;
use std::process::{Command, Output};

use log::{debug, error};

// Use crate::errors because errors.rs is a sibling module declared in lib.rs
use crate::errors::{GrabError, GrabResult};

/// Utility function to run an external command and capture its output.
/// Made crate-public as it's only needed internally by the listing module.
pub(crate) fn run_command(cmd: &str, args: &[&str], current_dir: &Path) -> GrabResult<Output> {
    debug!(
        "Running command: {} {:?} in directory: {:?}",
        cmd, args, current_dir
    );
    let output = Command::new(cmd)
        .args(args)
        .current_dir(current_dir) // Execute in the specified directory
        .output()
        // Map I/O errors during execution (like command not found)
        .map_err(|e| {
            let command_string = format!("{} {}", cmd, args.join(" "));
            if e.kind() == io::ErrorKind::NotFound {
                // Provide a specific error message if the command wasn't found
                error!(
                    "Command '{}' not found. Is '{}' installed and in your system's PATH?",
                    command_string, cmd
                );
            }
            // Wrap the error in our custom type
            GrabError::GitExecutionError {
                command: command_string,
                source: e,
            }
        })?;

    // Return the captured output (caller checks status code)
    Ok(output)
}

// No tests needed specifically for this module if covered by integration tests in lib.rs
