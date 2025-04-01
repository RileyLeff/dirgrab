# dirgrab

[![Crates.io](https://img.shields.io/crates/v/dirgrab.svg)](https://crates.io/crates/dirgrab) <!-- Update link/version later -->
[![Docs.rs](https://docs.rs/dirgrab-lib/badge.svg)](https://docs.rs/dirgrab-lib) <!-- Update link/version later -->
<!-- Add build status badge from GitHub Actions if you set that up -->

`dirgrab` is a simple command-line tool to grab the content of files within a directory and concatenate them, suitable for copying code snippets, project contexts, or feeding into language models.

It intelligently uses Git context when available (`git ls-files`) to only include tracked files (respecting `.gitignore`), but also works seamlessly on plain directories.

**Features:**

*   Concatenates file contents to stdout, a file (`-o`), or the clipboard (`-c`).
*   Uses `git ls-files` in Git repositories (respects `.gitignore` by default).
*   Optionally includes untracked files in Git repos (`-u`).
*   Works on non-Git directories by walking the file tree.
*   Allows custom exclude patterns (`-e`) using `.gitignore` glob syntax.
*   Includes file headers (`--- FILE: path/to/file ---`) by default (disable with `--no-headers`).
*   Skips binary/non-UTF8 files with a warning.

## Installation

Ensure you have Rust and Cargo installed. Then, install `dirgrab` using Cargo:

```bash
cargo install dirgrab
```

Verify the installation:

```bash
dirgrab --version
```

## Usage

```
dirgrab [OPTIONS] [TARGET_PATH]
```

**Arguments:**

*   `[TARGET_PATH]`: Optional path to the directory or Git repository to process. Defaults to the current working directory.

**Options:**

*   `-o, --output <FILE>`: Write output to a file instead of stdout.
*   `-c, --clipboard`: Copy output to the system clipboard.
*   `--no-headers`: Disable the default '--- FILE: ... ---' headers.
*   `-e, --exclude <PATTERN>`: Add glob patterns to exclude files/dirs (can be used multiple times).
*   `-u, --include-untracked`: [Git Mode Only] Include untracked files (still respects `.gitignore` and excludes).
*   `-v, -vv, -vvv`: Increase verbosity level for logging.
*   `-h, --help`: Print help information.
*   `-V, --version`: Print version information.

**Examples:**

1.  **Grab tracked files from current Git repo to stdout (default headers):**
    ```bash
    dirgrab
    ```

2.  **Grab files from current directory, disable headers, output to file:**
    ```bash
    dirgrab --no-headers -o output.txt
    ```

3.  **Grab tracked files from specific repo path, copy to clipboard:**
    ```bash
    dirgrab -c ../my-other-project/
    ```

4.  **Grab tracked + untracked files, excluding logs and build dirs:**
    ```bash
    dirgrab -u -e "*.log" -e "target/" -e "build/"
    ```

5.  **Grab all files from a non-Git directory, excluding temp files:**
    ```bash
    dirgrab -e "*.tmp" /path/to/non-git-dir
    ```

## Behavior Details

*   **Git Mode:** If the target directory is detected as part of a Git repository, `dirgrab` uses `git ls-files` to determine which files to include. This automatically respects `.gitignore` rules. The `-u` flag adds untracked files, still respecting `.gitignore`. Exclusions (`-e`) are passed to `git ls-files`.
*   **Non-Git Mode:** If the target directory is not a Git repository, `dirgrab` walks the directory tree. It includes all files found unless they match an exclusion pattern (`-e`). `.gitignore` files are *ignored* in this mode.
*   **File Encoding:** `dirgrab` attempts to read files as UTF-8. If a file cannot be decoded (likely binary), it is skipped, and a warning is printed to stderr (visible with `-v`).
*   **Output:** By default, file content is concatenated directly. With headers enabled (default), each file's content is preceded by `--- FILE: <relative_path> ---`. An extra newline is added between files for readability.

## Library (`dirgrab-lib`)

The core logic is available as a separate library crate (`dirgrab-lib`) for use in other Rust projects. See its documentation [on docs.rs](https://docs.rs/dirgrab-lib) (link will work after publishing).

## License

This project is licensed under either of:

*   Apache License, Version 2.0, (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)
*   MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT)

at your option.

## Contribution

Contributions are welcome! Please feel free to submit issues and pull requests.
<!-- Add more details if you have specific contribution guidelines -->