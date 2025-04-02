# dirgrab 📁⚡

[![Crates.io](https://img.shields.io/crates/v/dirgrab/0.2.0.svg)](https://crates.io/crates/dirgrab) <!-- Placeholder version -->
[![Docs.rs](https://docs.rs/dirgrab-lib/0.2.0/badge.svg)](https://docs.rs/dirgrab-lib) <!-- Placeholder version -->

`dirgrab` is a simple command-line tool to grab the contents of files within a directory and concatenate them, suitable for feeding whole-project contexts into language models.

It uses Git context when available (`git ls-files`) to only include tracked files (respecting `.gitignore`), but also works seamlessly on plain directories. By default, it includes a directory structure overview and file content headers.

**Features:**

*   Concatenates file contents to stdout, a file (`-o`), or the clipboard (`-c`).
*   Uses `git ls-files` in Git repositories (respects `.gitignore` by default).
*   Optionally includes untracked files in Git repos (`-u`).
*   Works on non-Git directories by walking the file tree.
*   Allows custom exclude patterns (`-e`) using `.gitignore` glob syntax.
*   Excludes `dirgrab.txt` by default (override with `--include-default-output`).
*   Includes a directory structure overview by default (disable with `--no-tree`).
*   Includes file headers (`--- FILE: path/to/file ---`) by default (disable with `--no-headers`).
*   Extracts text content from PDF files (disable with `--no-pdf`).
*   Optionally prints output size and word count to stderr (`-s`, `--stats`).
*   Skips binary/non-UTF8 files (other than convertible PDFs) with a warning.
*   Option to ignore Git context entirely (`--no-git`).
*   Core logic available as a library (`dirgrab-lib`).

## Installation

Ensure you have Rust and Cargo installed. Then, install `dirgrab` using Cargo:

```bash
cargo install dirgrab
# Or, to install/update from a local checkout:
# cargo install --path .
```

Verify the installation:

```bash
dirgrab --version
```

## Usage

```bash
dirgrab [OPTIONS] [TARGET_PATH]
```

**Arguments:**

*   `[TARGET_PATH]`: Optional path to the directory or Git repository to process. Defaults to the current working directory.

**Options:**

*   `-o, --output [FILE]`: Write output to a file. Defaults to `dirgrab.txt` in the current directory if `FILE` is omitted. Conflicts with `-c`.
*   `-c, --clipboard`: Copy output to the system clipboard. Conflicts with `-o`.
*   `--no-headers`: Disable the default '--- FILE: ... ---' headers.
*   `--no-tree`: Disable the default inclusion of the directory structure overview.
*   `-e, --exclude <PATTERN>`: Add glob patterns to exclude files/dirs (can be used multiple times).
*   `-u, --include-untracked`: [Git Mode Only] Include untracked files (still respects `.gitignore` and excludes). Has no effect if `--no-git` is used.
*   `--include-default-output`: Include `dirgrab.txt` if it exists (overrides default exclusion).
*   `--no-git`: Ignore Git context; treat target as a plain directory (disables `.gitignore` processing and the effect of `-u`).
*   `--no-pdf`: Disable the default extraction of text content from PDF files.
*   `-s, --stats`: Print output size (bytes) and word count to stderr upon completion.
*   `-v, -vv, -vvv`: Increase verbosity level for logging (Warn, Info, Debug, Trace).
*   `-h, --help`: Print help information.
*   `-V, --version`: Print version information.

**Examples:**

1.  **Grab tracked files from current Git repo to stdout (default tree and headers):**
    ```bash
    dirgrab
    ```

2.  **Grab files from current directory, disable tree and headers, output to file `context.txt`:**
    ```bash
    dirgrab --no-tree --no-headers -o context.txt
    ```

3.  **Grab tracked files, copy to clipboard, show stats:**
    ```bash
    dirgrab -c -s ../my-other-project/
    ```

4.  **Grab tracked + untracked files, excluding logs and build dirs, show stats:**
    ```bash
    dirgrab -u -e "*.log" -e "target/" -e "build/" -s
    ```

5.  **Grab all files from a non-Git directory, ignoring Git, excluding temp files, output to default `dirgrab.txt`:**
    ```bash
    dirgrab --no-git -e "*.tmp" /path/to/non-git-dir -o
    ```

6.  **Grab files, but force inclusion of `dirgrab.txt` if it exists:**
    ```bash
    dirgrab --include-default-output
    ```

## Behavior Details

*   **Git Mode (Default):** If the target directory is detected as part of a Git repository and `--no-git` is *not* used, `dirgrab` uses `git ls-files` to determine which files to include. This automatically respects `.gitignore` rules. The `-u` flag adds untracked files (still respecting `.gitignore`). User exclusions (`-e`) are applied. `dirgrab.txt` is excluded unless `--include-default-output` is used.
*   **Non-Git Mode (`--no-git` or not a repo):** `dirgrab` walks the directory tree. It includes all files found unless they match a user exclusion pattern (`-e`) or the default exclusion (`dirgrab.txt`, unless overridden). `.gitignore` files are *ignored*.
*   **Directory Tree:** By default, an indented directory tree showing the files *selected* for processing (after filtering) is prepended to the output. Use `--no-tree` to disable this.
*   **File Headers:** By default, each file's content is preceded by `--- FILE: <relative_path> ---`. The path is relative to the Git repo root (in Git mode) or the target path (in non-Git mode). Use `--no-headers` to disable.
*   **Default Output File:** The file `dirgrab.txt` is excluded by default to prevent accidental inclusion of previous runs. Use `--include-default-output` to override this specific exclusion.
*   **Output File (`-o`) Default:** If `-o` is provided without a subsequent filename, the output is written to `dirgrab.txt` in the current working directory.
*   **PDF Conversion:** Enabled by default, attempts to extract text from files ending in `.pdf` (case-insensitive). This relies on the `pdf-extract` crate and works best for text-based PDFs. Image-based PDFs or errors during extraction will result in a warning, and the file's content will be skipped (though a header might be added if enabled). Can be disabled with a feature flag.
*   **File Encoding:** For non-PDF files, `dirgrab` attempts to read files as UTF-8. If a file cannot be decoded (likely binary), it is skipped, and a warning is printed to stderr (visible with `-v`).
*   **Output Statistics (`-s`, `--stats`):** If enabled, the total byte count and a simple word count (based on whitespace) of the final generated output (including tree and headers) are printed to stderr after the operation completes successfully.

## Library (`dirgrab-lib`)

The core logic is available as a separate library crate (`dirgrab-lib`) for use in other Rust projects. The library structure has been refactored for better maintainability. See its documentation [on docs.rs](https://docs.rs/dirgrab-lib) (link will work after publishing).

## Changelog

### [0.2.0]

**Added**

*   PDF processing with the `pdf-extract` crate.
*   `-s`, `--stats` flag to optionally print output byte size and word count to stderr.
*   `--no-git` flag to force directory walking and ignore Git context even in a repository.
*   `--include-default-output` flag to specifically override the default exclusion of `dirgrab.txt`.
*   `--no-tree` flag to disable the directory tree overview (which is now enabled by default).

**Changed**

*   **BREAKING:** Directory tree structure is now included in the output **by default**. Use `--no-tree` to disable. (Previously required `--include-tree`).
*   `-o`/`--output` flag now defaults to writing to `dirgrab.txt` in the current directory if the flag is provided without an explicit filename.
*   Output statistics (byte/word count) are now **off by default**. Use `-s` or `--stats` to enable. (Previously always printed).
*   The file `dirgrab.txt` is now **excluded by default** in both Git and non-Git modes. Use `--include-default-output` to include it.
*   File headers for non-UTF8 files or files skipped due to errors are no longer added, improving clarity when content is omitted. PDF extraction failures may still add a specific "(PDF extraction failed)" header if headers are enabled.

**Internal**

*   Refactored `dirgrab-lib` into modules (`config`, `errors`, `listing`, `processing`, `tree`, `utils`) for better organization.
*   Added specific tests for PDF handling.
*   Resolved various Clippy warnings, including unnecessary unwraps.

### [0.1.0]

*   Initial release with core functionality:
    *   Directory walking and Git (`ls-files`) integration.
    *   Output to stdout, file (`-o <file>`), or clipboard (`-c`).
    *   Basic exclusion patterns (`-e`).
    *   Optional file headers (`--no-headers`).
    *   Optional inclusion of untracked files (`-u`).
    *   Verbose logging (`-v`).

## License

This project is licensed under either of:

*   Apache License, Version 2.0, (LICENSE-APACHE or [link](http://www.apache.org/licenses/LICENSE-2.0))
*   MIT license (LICENSE-MIT or [link](http://opensource.org/licenses/MIT))

at your option.

## Contribution

Contributions are welcome! Please feel free to submit issues and pull requests. Discuss significant changes via an issue first. Ensure code adheres to existing style (checked by `cargo fmt`) and passes Clippy (`cargo clippy`) and tests (`cargo test`).
