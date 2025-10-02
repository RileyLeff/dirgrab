Here’s a concrete, shippable plan for the next release of dirgrab (suggest 0.3.0) that incorporates the earlier review, adds global/local config and ignore files, improves Git-mode UX, and introduces “close-enough” token counting in stats.

Release scope and headline changes

New config system
Global and local config files and ignore files (TOML + .gitignore-like patterns).
Clear precedence: CLI > local config > global config > built-in defaults.
Flags to override/disable config loading.
Better Git-mode defaults
Include untracked files by default (still respects .gitignore).
Automatically scope to the target subtree when TARGET_PATH is inside a repo.
Deterministic ordering
Sorted file order for stable output.
Auto-exclude active output file
If you output to out.txt, it’s excluded automatically for that run.
Token stats (approximate, model-agnostic)
-s now reports bytes, words, and tokens with a simple, solid approximation.
Configurable token ratio and inclusion/exclusion of tree/headers for counting.
Reliability and polish
Fix release workflow source tarball bug.
Reduce default log noise for skipped binary files.
Robust git pathspecs for excludes.
Versioning

0.3.0 (minor breaking changes due to defaults in Git mode).
Detailed plan

Config and ignore files
File locations
Global config (TOML):
Linux: ~/.config/dirgrab/config.toml
macOS: ~/Library/Application Support/dirgrab/config.toml
Windows: %APPDATA%\dirgrab\config.toml
Global ignore (patterns): same dir as config: ignore
Local config (TOML): <target>/.dirgrab.toml
Local ignore (patterns): <target>/.dirgrabignore
Use the directories crate to find platform-appropriate global location. If both local and global exist, read both.
Precedence and merging
Start with built-in defaults.
If not --no-config:
Merge global TOML, then local TOML.
Merge CLI switches last (override everything).
Effective exclude patterns:
From merged TOML exclude list
Plus global ignore file lines (one per line)
Plus local ignore file lines
Plus -e patterns from CLI
Plus the active output file basename (if -o is used)
CLI controls
--no-config: ignore all config/ignore files
--config <path>: load TOML from explicit location (still apply defaults and ignore files unless --no-config)
Minimal TOML schema (may extend later)
[dirgrab]
exclude = ["Cargo.lock", "*.csv", "node_modules/", "target/"]
include_untracked = true
include_tree = true
add_headers = true
convert_pdf = true
include_default_output = false
no_git = false
tracked_only = false
all_repo = false
[stats]
enabled = true
token_ratio = 3.6
tokens_exclude = [] # allowed values: ["tree", "headers"]
Implementation notes
Add a small loader that returns an EffectiveConfig and Vec<String> effective_exclude_patterns.
Log at debug: which config/ignore files were found and how many patterns loaded.
2. Git-mode UX improvements

Defaults change (breaking):
include_untracked defaults to true (previously false).
If TARGET_PATH is inside a Git repo, limit file listing to that subtree by default.
New CLI flags
--tracked-only: sets include_untracked = false
--all-repo: disables subtree scoping; operate on entire repo
Listing behavior (git ls-files)
Tracked: git ls-files -z [scope] [excludes]
Untracked: git ls-files -z --others --exclude-standard [scope] [excludes]
Scope: if TARGET_PATH is inside the repo and not --all-repo, include a pathspec for that subdirectory relative to repo root.
Excludes: use pathspec magic for robustness: :(glob,exclude)pattern (instead of :!pattern).
Pathspec details
Scope pathspec: :(glob)<relative_subdir>/ (ensures recursive).
Exclude file/dir patterns are passed with :(glob,exclude) for Git.
3. Deterministic ordering

Sort the final list of files before processing (both git and walkdir modes). This stabilizes output and tests.
4. Auto-exclude the active output file

If -o <path> is used, add its basename to effective excludes (unless user explicitly included it).
This avoids ever picking up the freshly generated file on the next run.
5. Token stats (approximate)

Goal: “close-ish” model-agnostic estimate, no heavy dependencies required.
Approach: configurable ratio-based estimate:
tokens ≈ ceil( num_chars / token_ratio ), default token_ratio = 3.6
Provide a config knob and CLI override for token_ratio, e.g. --token-ratio <float>
Include in -s output: “tokens≈N (ratio=R)”
Scope options for counting
By default, count over the exact output string (includes tree and headers).
Allow excluding these from the token count via config or CLI:
config [stats].tokens_exclude = ["tree", "headers"]
CLI flags: --tokens-exclude-tree, --tokens-exclude-headers
Implement by keeping parallel strings or recomputing count over selected slices.
Output example with -s
Output Size (to stdout): 12345 bytes, 2345 words, tokens≈3400 (ratio=3.6)
6. Reliability and polish

Release workflow fix (critical)
Replace the xargs tar invocation with a --null file list:
{ git ls-files -z; printf "%s\0" README.md LICENSE* .gitignore .pre-commit-config.yaml Cargo.lock; } | tar --null -czf "$ARCHIVE_NAME" -T -
Lower log level for skipped non-UTF8
Change warn!("Skipping non-UTF8…") to info or debug to reduce noise.
Processing optimization
Replace buffer.clone and BufReader with fs::read + String::from_utf8 (or fs::read_to_string) to avoid extra allocation/copy.
Tests
Update tests for new defaults (untracked included and subtree scoping).
Sort expectations to match deterministic ordering.
PDF test: skip gracefully if sample.pdf fixture is missing, or add a small fixture to the repo.
Pre-commit fix
Ensure both crate READMEs are added in the sync hook.
CLI summary (post-change)

Existing:
-o/--output [FILE], -c/--clipboard, --no-headers, --no-tree, -e/--exclude <PATTERN>, --include-default-output, --no-git, --no-pdf, -s/--stats, -v
New:
--tracked-only
--all-repo
--no-config
--config <FILE>
--token-ratio <FLOAT> (default 3.6)
--tokens-exclude-tree
--tokens-exclude-headers
Library/public API changes

GrabConfig additions/changes:
include_untracked: default true (handled by CLI/config merge; the struct remains a bool)
all_repo: bool
tracked_only: bool (or compute include_untracked = !tracked_only in CLI before constructing GrabConfig)
listing::list_files_git(repo_root, config, scope_subdir: Option<&Path>) -> Vec<PathBuf>
Add scope_subdir param.
Ensure sorting inside the function or sort at the call site.
list_files_walkdir: sort before returning.
processing::process_files: no signature change; internally optimize reads and lower log level for non-UTF8.
Config loader module

Add a small module (e.g., dirgrab-lib/src/config_load.rs or in the binary crate) that:
Locates config and ignore files.
Parses TOML if present (serde + toml).
Reads ignore files into Vec<String>.
Builds EffectiveConfig and effective_exclude_patterns.
Applies auto-exclusion for the active output file basename (if provided).
Returns a final GrabConfig and patterns for listing (or just a final GrabConfig with the merged exclude_patterns baked in).
Keep the loader thin and well-logged at debug.
Documentation updates

README
Document config/ignore files, precedence, examples.
Provide a sample ~/.config/dirgrab/config.toml with common defaults (Cargo.lock, *.csv, node_modules/, target/).
Explain -s now includes approximate tokens; show how to adjust --token-ratio and exclude tree/headers from the token count.
Mention new defaults: includes untracked, subtree scoping in Git mode.
Update badges to version-agnostic badges.
Changelog (0.3.0)
Added: global/local config and ignore files; token stats (approx).
Changed (breaking): Git-mode defaults (untracked included; subtree scoping).
Fixed: deterministic ordering; release tar creation; reduced log noise; active output auto-excluded.
Internal: stronger Git pathspecs; simplified file reading.
Acceptance criteria

Config
With only global config specifying exclude = ["Cargo.lock", "*.csv"], running dirgrab in any project excludes those patterns without -e flags.
Local .dirgrabignore overrides apply only to that project and merge with global.
--no-config ignores both config and ignore files.
Git defaults
In a repo, dirgrab subdir now includes files only from that subdir, includes untracked, respects .gitignore, and applies excludes.
--tracked-only excludes untracked.
--all-repo includes the entire repo again.
Deterministic order
Running twice yields identical output (no reordering).
Stats
-s prints bytes, words, and tokens≈ with the configured ratio.
Adjusting --token-ratio changes the token estimate accordingly.
--tokens-exclude-tree/--tokens-exclude-headers affect token count but not the emitted output.
Release pipeline
Source tarball contains the full repo content per git ls-files plus the listed top-level files; no partial archives.
Timeline and risk

Low-risk, incremental merge path:
Deterministic sort, processing optimization, lowered logs, auto-exclude active output, and release.yml fix.
Git-mode scoping + new defaults + flags (--tracked-only, --all-repo) with tests.
Config/ignore loader + precedence + doc updates.
Token stats (approx) + flags + tests.
Optional future work (not required for 0.3.0)
Feature-flag PDF extractor dependency.
“Diff-only” modes (only changed files).
Max total bytes/file size caps and extension filters.
Provider-specific tokenizers (OpenAI/Claude/Gemini), if ever desired.
Sample global config to share in README
~/.config/dirgrab/config.toml

[dirgrab]
exclude = ["Cargo.lock", "*.csv", "node_modules/", "target/"]
include_untracked = true
include_tree = true
add_headers = true
convert_pdf = true
include_default_output = false
no_git = false
tracked_only = false
all_repo = false

[stats]
enabled = true
token_ratio = 3.6
tokens_exclude = ["tree"] # count tokens on headers+contents only
