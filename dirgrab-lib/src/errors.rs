// --- FILE: dirgrab-lib/src/errors.rs ---

use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during the `dirgrab` library operations.
///
/// These errors cover issues ranging from file system access problems
/// to Git command failures and configuration errors.
#[derive(Error, Debug)]
pub enum GrabError {
    // Make enum public
    /// The initial `target_path` provided in the `GrabConfig` was not found
    /// on the filesystem or was inaccessible due to permissions.
    #[error("Target path not found or not accessible: {0}")]
    TargetPathNotFound(PathBuf),

    /// An I/O error occurred while accessing a path during the operation
    /// (e.g., reading a file, canonicalizing a path).
    #[error("IO error accessing path '{path}': {source}")]
    IoError {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A `git` command (like `git ls-files` or `git rev-parse`) failed to execute
    /// successfully, indicated by a non-zero exit status.
    /// Contains the command string, stderr, and stdout output for debugging.
    #[error("Failed to execute git command: {command:?}\n  stderr: {stderr}\n  stdout: {stdout}")]
    GitCommandError {
        command: String,
        stderr: String,
        stdout: String,
    },

    /// An error occurred while trying to spawn or run the `git` process itself.
    /// This commonly happens if `git` is not installed or not found in the system's PATH,
    /// but can also indicate permission errors preventing execution.
    #[error("Failed to run git command '{command}': {source}")]
    GitExecutionError {
        command: String,
        #[source]
        source: io::Error,
    },

    /// Failed to build the glob pattern matcher from the patterns provided
    /// in `GrabConfig::exclude_patterns`. This might happen if a pattern has
    /// invalid syntax according to the `ignore` crate.
    #[error("Failed to build glob pattern matcher: {0}")]
    GlobMatcherBuildError(#[source] ignore::Error),

    /// Error specifically for path stripping issues during tree generation.
    #[error("Failed to strip prefix '{prefix}' from path '{path}' during tree generation")]
    PathStripError { prefix: PathBuf, path: PathBuf },
}

/// A convenience type alias for `Result<T, GrabError>`.
pub type GrabResult<T> = Result<T, GrabError>; // Make type alias public
