// --- FILE: dirgrab-lib/src/config.rs ---

use std::path::PathBuf; // Needed for the struct definition

/// Configuration for the dirgrab operation.
///
/// This struct holds all the settings needed to control how `dirgrab`
/// finds and processes files within the specified target directory.
/// It is typically constructed by the calling application (e.g., the CLI)
/// based on user input.
#[derive(Debug, Clone)]
pub struct GrabConfig {
    // Made public here
    /// The starting path for the operation (directory or Git repository).
    /// `dirgrab` will operate within this path. It will be canonicalized internally.
    pub target_path: PathBuf,

    /// If true, adds `'--- FILE: <filename> ---'` headers before each file's content
    /// in the final output string. The filename displayed will be relative to the
    /// Git repository root (if applicable) or the target path.
    pub add_headers: bool,

    /// A list of glob patterns (using .gitignore syntax) to exclude files or directories.
    /// These patterns are applied *in addition* to any `.gitignore` rules if operating
    /// in Git mode.
    /// In Git mode, they are passed to `git ls-files` as `:!<pattern>` pathspecs.
    /// In non-Git mode, they are used to filter the results from walking the directory.
    pub exclude_patterns: Vec<String>,

    /// If operating in Git mode, set this to true to include untracked files
    /// (files present in the working directory but not added to the index).
    /// This still respects `.gitignore` and the `exclude_patterns`.
    /// This setting has no effect if the `target_path` is not part of a Git repository,
    /// or if `no_git` is true.
    pub include_untracked: bool,

    /// If true, the default exclusion for `dirgrab.txt` will *not* be applied.
    /// Use this flag only if you specifically need to include a file named `dirgrab.txt`.
    pub include_default_output: bool,

    /// If true, forces dirgrab to ignore any Git repository context and treat the
    /// target path purely as a filesystem directory. This disables `.gitignore`
    /// processing and the effect of `include_untracked`. User-provided exclude
    /// patterns (`-e`) are still respected.
    pub no_git: bool,

    /// If true, prepend an indented directory tree structure to the output,
    /// showing the files and directories included in the grab operation.
    pub include_tree: bool,

    /// If true, attempt to extract text content from PDF files.
    pub convert_pdf: bool, // <-- Field added here

    /// If true, operate on the entire Git repository even when the target path is a subdirectory.
    pub all_repo: bool,
}
