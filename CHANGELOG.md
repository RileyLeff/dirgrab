# Changelog

All notable changes to this project are tracked here. New releases follow
semantic versioning (major.minor.patch). For details on upcoming work, check
open issues and milestones.

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
