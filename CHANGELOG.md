# Changelog

All notable changes to this project are tracked here. New releases follow
semantic versioning (major.minor.patch). For details on upcoming work, check
open issues and milestones.

## [0.4.0] - 2026-02-11

### Breaking Changes

- **`-e/--exclude` no longer greedily consumes multiple arguments.** Each `-e`
  takes exactly one value. Use comma-separated patterns for convenience:
  `-e "*.log,target/,*.tmp"`. Previously, `dirgrab -e foo bar` treated both
  `foo` and `bar` as exclude patterns; now `bar` is correctly interpreted as
  the target path.
- **`--tracked-only` and `-u/--include-untracked` now conflict.** Passing both
  produces a clear error instead of silently letting `-u` win.

### New Features

- Added `-l/--list` flag for dry-run file listing. Prints one file path per
  line without reading contents — useful for verifying exclude patterns before
  a full grab.
- Added `list_files()` public API to `dirgrab-lib` for programmatic file
  listing without content processing.
- Output format is now documented in `--help` (via `after_long_help`).

### Bug Fixes

- Fixed symlink handling in `--no-git` mode: symlinks are now followed,
  matching Git mode behavior. Circular symlinks are detected and skipped.
- Fixed PDF extraction failure producing inconsistent segment metadata
  (header/body byte ranges now match the structure of successful files).
- Made `strip_header_lines` (used for `--tokens-exclude-headers`) more
  robust: only strips lines matching both the `--- FILE: ` prefix and
  ` ---` suffix, preventing false positives in file content.
- Deduplicated `normalize_glob` across library and binary crates.
- Added warnings when `--all-repo` or `--tracked-only` are used with
  `--no-git` (these flags have no effect in plain directory mode).
- Documented that `-o` auto-excludes the output filename from the grab.

## [0.3.2] - 2025-02-15

- Fixed `-e/--exclude` so a single flag can absorb every shell-expanded
  argument (e.g., `-e integration_tests/*`), avoiding clap parse errors when
  globbing produces multiple paths.
- Documented quoting expectations for exclude globs and added Homebrew install
  instructions in the READMEs.
- Ran `cargo update` to resolve the warning about the yanked `deranged` crate
  and refresh other transitive dependencies.

## [0.3.1] - 2025-02-14

- Expanded `-s/--stats` into a multi-report flag with defaults for totals and
  `top-files=5`, plus support for selecting custom report bundles (e.g.
  `--stats overview top-files=10`).
- Added a per-file token leaderboard to the CLI stats output and exposed
  detailed file metadata via the new `grab_contents_detailed` API in
  `dirgrab-lib`.

## [0.3.0]

- Added layered configuration (global `config.toml`/`ignore`, project
  `.dirgrab.toml`/`.dirgrabignore`, CLI precedence).
- Git mode now scopes to the requested subtree, includes untracked files by
  default, and exposes `--tracked-only` / `--all-repo` switches.
- Ensured deterministic file ordering and automatic exclusion of the active
  output file.
- Extended `--stats` with configurable token estimates (`--token-ratio`,
  `--tokens-exclude-*`).
- Reduced binary-file log noise and streamlined file reading for better
  performance.
- Fixed the release workflow to generate source archives via null-delimited tar
  input.

## [0.2.0]

- Added PDF extraction, directory tree by default, `--no-git`,
  `--include-default-output`, `--no-tree`, and optional stats output.
- Defaulted the output filename for `-o`, skipped non-UTF8 headers, and
  refactored the library layout.

## [0.1.0]

- Initial release: Git-aware file selection, clipboard and file outputs,
  excludes, headers, and verbosity controls.
