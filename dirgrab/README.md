# dirgrab 📁⚡

[![Crates.io](https://img.shields.io/crates/v/dirgrab.svg)](https://crates.io/crates/dirgrab)
[![Docs.rs](https://docs.rs/dirgrab-lib/badge.svg)](https://docs.rs/dirgrab-lib)

`dirgrab` walks a directory (or Git repository), selects the files that matter, and concatenates their contents for easy copy/paste into language models. It can write to stdout, a file, or your clipboard, and it ships with a library crate so the same logic can be embedded elsewhere.

## Highlights

- 🔧 **Configurable defaults** – merge built-in defaults with global `config.toml`, project-local `.dirgrab.toml`, `.dirgrabignore`, and CLI flags.
- 🧭 **Git-aware out of the box** – untracked files are included by default, scoped to the selected subdirectory, with `--tracked-only` and `--all-repo` to opt out.
- 🗂️ **Structured context** – optional directory tree, per-file headers, PDF text extraction, and deterministic file ordering for stable diffs.
- 🧮 **Better stats** – `-s/--stats` now prints bytes, words, and an approximate token count with configurable ratio and exclusion toggles.
- 🙅 **Safety nets** – automatically ignores the active output file, respects `.gitignore`, and gracefully skips binary/non-UTF8 files.

## Installation

```bash
cargo install dirgrab
# or from a local checkout
# cargo install --path .
```

Check it worked:

```bash
dirgrab --version
```

## Usage

```bash
dirgrab [OPTIONS] [TARGET_PATH]
```

`TARGET_PATH` defaults to the current directory. When invoked inside a Git repo, `dirgrab` scopes the listing to that subtree unless you pass `--all-repo`.

### Common Options

- `-o, --output [FILE]` – write to a file (defaults to `dirgrab.txt` if no name is given). Conflicts with `--clipboard`.
- `-c, --clipboard` – copy to the system clipboard instead of stdout or a file.
- `--no-headers` / `--no-tree` / `--no-pdf` – disable headers, the directory tree, or PDF extraction.
- `-e, --exclude <PATTERN>` – add glob-style excludes (applied after config files).
- `--tracked-only` – Git mode: limit to tracked files. (Compatibility note: `-u/--include-untracked` still forces inclusion if you need it.)
- `--all-repo` – Git mode: operate on the entire repository even if the target is a subdirectory.
- `--include-default-output` – allow `dirgrab.txt` back into the run.
- `--no-git` – ignore Git context entirely and walk the filesystem.
- `--no-config` – ignore global/local config files and `.dirgrabignore`.
- `--config <FILE>` – load an additional TOML config file (applied after global/local unless `--no-config`).
- `--token-ratio <FLOAT>` – override the characters-to-tokens ratio used by `--stats` (defaults to 3.6).
- `--tokens-exclude-tree` / `--tokens-exclude-headers` – subtract tree or header sections when estimating tokens.
- `-s, --stats` – print bytes, words, and approximate tokens to stderr when finished.
- `-v, -vv, -vvv` – increase log verbosity (Warn, Info, Debug, Trace).
- `-h, --help` / `-V, --version` – CLI boilerplate.

### Configuration Files

`dirgrab` layers configuration in the following order (later wins):

1. Built-in defaults
2. Global config + ignore
   - Linux: `~/.config/dirgrab/config.toml` & `~/.config/dirgrab/ignore`
   - macOS: `~/Library/Application Support/dirgrab/config.toml` & `…/ignore`
   - Windows: `%APPDATA%\dirgrab\config.toml` & `ignore`
3. Project-local config: `<target>/.dirgrab.toml`
4. Project-local ignore patterns: `<target>/.dirgrabignore`
5. CLI flags (`--tracked-only`, `--no-tree`, etc.)

Sample `config.toml`:

```toml
[dirgrab]
exclude = ["Cargo.lock", "*.csv", "node_modules/", "target/"]
include_tree = true
add_headers = true
convert_pdf = true
tracked_only = false
all_repo = false

[stats]
enabled = true
token_ratio = 3.6
tokens_exclude = ["tree"]
```

`ignore` files use the same syntax as `.gitignore`. CLI `-e` patterns and the active output file name are appended last, so the freshly written file is never re-ingested accidentally.

### Examples

```bash
# Grab the current repo subtree (includes untracked files) and show stats
dirgrab -s

# Limit to tracked files only and exclude build artifacts
dirgrab --tracked-only -e "*.log" -e "target/"

# Force a whole-repo snapshot from within a subdirectory
dirgrab --all-repo

# Plain directory mode with custom excludes, writing to the default file
dirgrab --no-git -e "*.tmp" -o

# Use project defaults but ignore configs for a “clean” run
dirgrab --no-config --no-tree --no-headers
```

## Behaviour Notes

- **Git scope & ordering** – Paths are gathered via `git ls-files`, scoped to the target subtree unless `--all-repo` is set, and the final list is sorted for deterministic output. Non-Git mode uses `walkdir` with the same ordering.
- **File headers & tree** – Headers and tree sections remain enabled by default; toggle them per run or through config files.
- **PDF handling** – Text is extracted from PDFs unless disabled. Failures and binary files are skipped with informative (but less noisy) logs.
- **Stats** – When `--stats` is active (or enabled in config), stderr shows bytes, words, and an approximate token count. Exclude tree/headers or change the ratio via config or CLI.
- **Safety** – `dirgrab.txt` stays excluded unless explicitly re-enabled, and any active `-o FILE` target is auto-excluded for that run.

## Library (`dirgrab-lib`)

The same engine powers `dirgrab-lib`; import it to drive custom tooling:

```rust
use dirgrab_lib::{grab_contents, GrabConfig};
# // build a GrabConfig and call grab_contents(&config)
```

See [docs.rs](https://docs.rs/dirgrab-lib) for API details.

## Changelog

### [0.3.0]

- Added layered configuration (global `config.toml`/`ignore`, project `.dirgrab.toml`/`.dirgrabignore`, CLI precedence).
- Git mode now scopes to the requested subtree, includes untracked files by default, and exposes `--tracked-only` / `--all-repo` switches.
- Ensured deterministic file ordering and automatic exclusion of the active output file.
- Extended `--stats` with configurable token estimates (`--token-ratio`, `--tokens-exclude-*`).
- Reduced binary-file log noise and streamlined file reading for better performance.
- Fixed the release workflow to generate source archives via null-delimited tar input.

### [0.2.0]

- Added PDF extraction, directory tree by default, `--no-git`, `--include-default-output`, `--no-tree`, and optional stats output.
- Defaulted the output filename for `-o`, skipped non-UTF8 headers, and refactored the library layout.

### [0.1.0]

- Initial release: Git-aware file selection, clipboard and file outputs, excludes, headers, and verbosity controls.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

## Contributing

Issues and PRs are welcome! Please run `cargo fmt`, `cargo clippy`, and `cargo test` before submitting.
