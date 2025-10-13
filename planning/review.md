Here’s a thorough review of dirgrab focusing on correctness, cleanup, simplification, and a concrete path to the features you want (A–C). I’ve anchored notes to files and gave actionable patches where it helps.

High-impact bugs and inconsistencies

tokens_exclude_tree doesn’t fully exclude the “FILE CONTENTS” header

Where: dirgrab/src/main.rs, strip_tree_section()
Issue: When excluding the tree from token-basis, you slice from the start of the header instead of after it. This keeps the separator in the token count.
Fix:
Return content[idx + FILE_CONTENTS_HEADER.len()..] instead of content[idx..].
Also handle the “no files selected” case gracefully (no header present).
Patch:
fn strip_tree_section(content: &str) -> String {
    const FILE_CONTENTS_HEADER: &str = "---\nFILE CONTENTS\n---\n\n";
    if let Some(idx) = content.find(FILE_CONTENTS_HEADER) {
        content[idx + FILE_CONTENTS_HEADER.len()..].to_string()
    } else {
        // If there's no FILE CONTENTS header, just return the original content.
        // This avoids returning "" and undercounting.
        content.to_string()
    }
}
Misleading IoError path in detect_git_repo

Where: dirgrab-lib/src/listing.rs, detect_git_repo()
Issue: On canonicalize failure, you wrap IoError with path set to "detected git root". That makes debugging harder.
Fix: Use the actual stdout-derived root path in the IoError or stderr context.
Suggested change:
let root_path_raw = PathBuf::from(&stdout);
let root_path = root_path_raw.canonicalize().map_err(|e| GrabError::IoError {
    path: root_path_raw.clone(),
    source: e,
})?;
Walkdir mode doesn’t prune ignored directories

Where: dirgrab-lib/src/listing.rs, list_files_walkdir()
Issue: You only filter files after traversal. Patterns like target/ or node_modules/ don’t prune descent, causing unnecessary IO on large trees.
Fix: Use WalkDir::into_iter().filter_entry() to skip ignored directories when matched_path_or_any_parents(dir, true) == Match::Ignore(_).
Sketch:
let matcher = exclude_builder.build()?;
let it = WalkDir::new(target_path).into_iter().filter_entry(|e| {
    if e.file_type().is_dir() {
        !matches!(matcher.matched_path_or_any_parents(e.path(), true), Match::Ignore(_))
    } else {
        true
    }
});
for entry in it {
    let entry = match entry { Ok(e) => e, Err(e) => { ...; continue; } };
    if !entry.file_type().is_file() { continue; }
    match matcher.matched_path_or_any_parents(entry.path(), false) {
        Match::None | Match::Whitelist(_) => files.push(entry.into_path()),
        Match::Ignore(_) => {}
    }
}
Windows-style backslashes in exclude patterns

Where: dirgrab-lib/src/listing.rs
Issue: For git pathspecs, patterns must use forward slashes. If a user passes a Windows-style pattern like subdir\ or *.log with backslashes, matches can fail.
Fix: Normalize exclude patterns by replacing '' with '/' in build_exclude_pathspecs() and when constructing walkdir ignore patterns.
Suggest:
fn normalize_glob(p: &str) -> String { p.replace('\\', "/") }
// Apply normalize_glob() to every pattern before using it anywhere (git or walkdir).
Treating “fatal: detected dubious ownership” as “not a repo”

Where: dirgrab-lib/src/listing.rs, detect_git_repo()
Issue: You return Ok(None). That silently drops to walkdir, masking a security-config issue that a user may want to fix.
Recommendation: Warn prominently and hint at git config --global --add safe.directory <path>. Keep the fallback, but at warn level (not debug).
Example:
if stderr.contains("detected dubious ownership") {
    warn!("Git reports 'dubious ownership' at {:?}. Falling back to non-git mode. Consider: git config --global --add safe.directory {:?}", path, path);
    return Ok(None);
}
docs duplication in lib.rs

Where: dirgrab-lib/src/lib.rs
Issue: #[doc = include_str!("../README.md")] at crate level and also on grab_contents duplicates the entire readme twice in docs.rs.
Fix: Keep the crate-level include; remove the include on grab_contents.
Pre-commit sync hook only adds one README

Where: .pre-commit-config.yaml
Issue: The Sync workspace README hook runs cp to both crates but git add only dirgrab-lib/README.md. This can leave dirgrab/README.md unstaged.
Fix: add dirgrab/README.md to git add.
Patch:
entry: bash -c 'cp README.md dirgrab-lib/README.md && cp README.md dirgrab/README.md && git add dirgrab-lib/README.md dirgrab/README.md'
Release workflow: repository URL

Where: .github/workflows/release.yml
Issue: You use ${{ github.repository_url }} which isn’t a default context var. Prefer https://github.com/${{ github.repository }}.
Example:
url "https://github.com/${{ github.repository }}/releases/download/${{ github.ref_name }}/${{ needs.create_source_archive.outputs.archive_name }}"
Small test hygiene

Duplication: tests define run_test_command even though the utils::run_command exists. Either reuse utils::run_command for consistency or keep this helper but mark it as local-only to tests to avoid confusion.
Git presence: setup_git_repo already checks for git; nice.
Dead/unused error variants

Where: dirgrab-lib/src/errors.rs
Likely unused: RepoRootNotFound, WalkdirError, NonUtf8File.
Either remove them or use them. Right now you always log and skip rather than propagate these error types.
Path-dependent header behavior is correct but brittle for future tree changes

The headers logic depends on repo_root presence to compute relative paths. This is OK. Just document the invariant: in git mode, headers are repo-root relative (even if scoped); in no-git mode, headers are target-path relative.
Design simplifications and refactors

Refactor processing into a pipeline with typed outputs

Current processing.rs fuses reading, PDF extraction, headers, and string building. To support per-file stats (feature A), preset handlers (B), and LLM hooks (C), introduce an internal abstraction:
struct ProcessedFile { path_rel: PathBuf, bytes: usize, chars: usize, words: usize, tokens_est: usize, content: String, kind: FileKind }
enum FileKind { Text, PdfExtracted, BinarySkipped, Hook(String) }
A Processor trait or just a set of functions that either produce ProcessedFile or skip with a reason.
grab_contents can then:
build Vec<ProcessedFile>
render tree
render files (adding headers if needed)
compute stats easily, and implement “top N” by tokens without re-reading.
Consolidate exclude normalization

Ensure a single helper (normalize_glob) is used consistently before pushing patterns into either Git pathspec or ignore::GitignoreBuilder. Then you don’t have to special-case separators in two places.
Optionally feature-gate pdf-extract

Add a cargo feature pdf (default = true) in dirgrab-lib. This lets minimal users avoid pulling in pdf-extract and its graph.
[features] pdf = [] ; default = ["pdf"]
cfg-if around the pdf code path. When disabled, treat PDFs as binary/non-UTF8 and skip or require a hook.
Prefer read_to_string for text and fall back if decoding fails

Your current fs::read + String::from_utf8 is fine and avoids double buffers. If you implement per-file handlers, you can cleanly separate “read bytes” vs “decode string” responsibilities.
Sorting

You already sort in list_files_git and walkdir mode. Good.
Consider sorting by a stable relative path (repo-root relative for git; target-path relative for walkdir) so header paths and sorting use the same base.
API ergonomics

dirgrab depends on dirgrab-lib via path and version. In a workspace, you can drop the explicit version in dirgrab’s Cargo.toml for less churn:
dirgrab-lib = { path = "../dirgrab-lib" }
Feature A: top-N by tokens/bytes/words

Goal

A CLI option to rank files by tokens (or bytes/words) to identify candidates to exclude.
Design

Add a new CLI group:
--rank top by tokens (default), bytes, or words
--top N (default maybe 10)
--rank-only (don’t emit content), or combine with normal run to still write content and print the table on stderr.
Implementation:
After listing files, pre-process each file into ProcessedFile (see refactor). For PDFs, reuse the same extraction path so the token estimate matches reality.
Estimated tokens can use the same ratio and exclusion flags (headers don’t matter per-file; tree is irrelevant). Use chars.count()/ratio for each file’s content only.
Output:
Simple text table to stderr:
Rank Path Bytes Words Tokens
Add --rank-by bytes|words|tokens and --descending/--ascending.
Minimal code sketch (after refactor):

struct FileStat { rel: PathBuf, bytes: usize, words: usize, tokens: usize }
fn rank_files(processed: &[ProcessedFile], by: RankBy) -> Vec<FileStat> { ... }
Feature B: presets and hooks per file type

Goal

Allow users to define how to handle specific file types or patterns. Examples:
CSV: take header + first 10 rows
Large JSON: show first N keys / summarize
Images: skip or OCR
Also support “external command” hooks to transform file content, so users can shell out or use Python (PEP 723, DSPy, etc.).
Config schema

Extend TOML with a new section:
[handlers]
# Order matters; first match wins
# Pattern is a glob (.gitignore style). Internally normalize to forward slashes.
[[handlers.rule]]
pattern = "*.csv"
preset = "csv_head"
rows = 10
include_headers = true

[[handlers.rule]]
pattern = "*.md"
command = ["bash", "-lc", "sed -n '1,200p' {path}"]

[[handlers.rule]]
pattern = "data/**/*.json"
preset = "json_head"
keys = 100

[[handlers.rule]]
pattern = "*.pdf"
preset = "pdf_extract"  # default
# or use an external LLM hook:
command = ["python3", "/path/to/pdf_llm.py", "--file", "{path}"]
timeout_ms = 30000
Resolution
Evaluate in order; first matching rule handles the file.
Built-in presets: pdf_extract (default), csv_head, text (default), skip (do nothing).
command hook:
Interpolate placeholders: {path}, {repo_root}, {target_path}, {relpath}, {ext}, {mime}, {size}, {env:FOO}, {cfg:...}
Capture stdout as file content; stderr goes to debug logs.
Support timeout.
Security: warn users that external hooks execute arbitrary code.
Implementation

Introduce a simple Handler abstraction in dirgrab-lib:
trait Handler {
    fn name(&self) -> &str;
    fn handles(&self, path: &Path) -> bool; // or provide pattern externally
    fn process(&self, ctx: &Context, file: &Path) -> Result<Option<String>, GrabError>;
}
Provide a Registry built from config, injecting built-in handlers and external-command handlers configured from TOML.
processing.rs uses Registry to select a handler for each file. Fallback to TextHandler then BinarySkip.
CSV head preset:
Use streaming reading, write headers and first N rows.
Consider using csv crate; otherwise, a simple line-based approach for “first 10 lines” is ok but won’t handle quoted fields. Since this is a “preset”, a dependency on csv = "1" is reasonable.
JSON head preset:
Optional and future.
Feature C: LLM integration (DSPy or others) as modular hooks

Goal

Allow LLM-backed parsing or summarization, e.g., for PDFs or large files; keep default local PDF extractor but allow users to opt into LLM hooks.
Approach

Don’t bake DSPy (Python) into Rust. Expose LLM hooks via the same external command mechanism above.
Provide an “official” example hook repo (or folder in this repo’s examples/) with Python scripts using DSPy or other SDKs (OpenAI, Gemini, Claude). These scripts:
Accept file path and/or stdin as input.
Output extracted or summarized text to stdout.
Read API keys from env vars.
Avoid sending gigantic content by chunking on their side (script’s responsibility).
Rust side:
Add a “llm” feature that includes helper utilities (timeout, environment passthrough, maybe a simple retry).
Add a preset name like preset = "pdf_llm_gemini" that expands into command = ["python3", ".../gemini_pdf.py", "{path}"] when the llm feature is enabled, or emit a helpful error if feature not enabled.
Caching:
Optional: if the handler is expensive, cache results keyed by file path + mtime + size + handler ID in a cache dir (~/.cache/dirgrab). This will speed repeated runs.
Other improvements you’ll likely want soon

Maximum size limits and truncation

Add [limits] section:
[limits]
max_file_bytes = 500000
max_total_bytes = 5000000
Let handlers respect max_file_bytes (truncate with a suffix like “… [truncated]”).
Whitelisting patterns (negations)

Your underlying ignore crate supports “!” patterns. Document that users can put “!keep_this.txt” in ignore files. Right now the loader only “pushes” patterns, which is fine; matched_path_or_any_parents already interprets negations.
Better stats mode

Add --per-file-stats to print per-file stats for all files; --top and --rank-by naturally apply to this view.
Security and portability for command hooks

On Windows, prefer Command with explicit args rather than a shell. Offer a cross-platform placeholder:
command_shell = true means “run through system shell”
else run command directly without a shell
Always provide a timeout and a max stdout size to avoid memory bombs.
Logging levels

Your non-UTF8 skip logs are at info; that’s good to reduce noise from binaries. Consider bumping read errors to warn (as you do).
CLI UX

Add a dry-run mode: --ls prints the final file list (paths), with optional --json to integrate into scripts.
CI/workflows

You’ve already fixed the release tar null-delimited input. Nice.
Cross-compile matrix looks sane for now.
Minor polish

list_files_git: consider logging the final number of unique files after merging tracked/untracked, and the split counts (tracked N, untracked M) when debug-enabled.
derive_scope_subdir: If target is a file (user points at a single file), you might want to support that by scoping to just that file. Currently, the code canonicalizes, and you later use list_files for the whole subtree. Consider an explicit “if the target path is a file, only process that file” branch early in grab_contents.
In README and help: mention that “pattern semantics are .gitignore-like; use forward slashes; negations with !pattern are allowed”.
strip_header_lines: This removes only the line that starts with “--- FILE: ”. If you later change header format, this may become brittle. That’s fine for now but consider keeping a single source of truth for header markers.
Suggested patches (quick wins)

Fix strip_tree_section
As above under Bugs.

Normalize exclude patterns for git and walkdir
Add this helper and apply it:

fn normalize_glob(p: &str) -> String { p.replace('\\', "/") }
Apply in:
PatternAccumulator.push() before inserting
build_exclude_pathspecs()
list_files_walkdir() before add_line
Prune directories in walkdir mode
As above under Bugs; use filter_entry.

Improve dubious ownership handling in detect_git_repo
As above under Bugs; log warn with remediation suggestion.

Remove unused error variants

Remove RepoRootNotFound, WalkdirError, NonUtf8File unless you will start using them.
Run clippy afterwards (already enforced) to ensure no fallout.
Remove duplicate include_str docs
Keep only crate-level include_str! in lib.rs.

Pre-commit hook
Add dirgrab/README.md to git add.

Release workflow URL
Replace ${{ github.repository_url }} with https://github.com/${{ github.repository }}.

Feature implementation plan (incremental, low-risk order)

Phase 1: internal refactor + top-N

Introduce ProcessedFile and a small processing registry:
text (default), pdf_extract (existing), binary-skip.
Make processing.rs return Vec<ProcessedFile> (or iterate yield -> collect in grab_contents).
Compute per-file stats and expose via new API:
pub struct FileStat { rel: PathBuf, bytes, words, tokens }
pub fn analyze(config: &GrabConfig) -> GrabResult<(Vec<FileStat>, String)> // stats and rendered content
Add CLI: --top 10 --rank-by tokens|bytes|words --rank-only
Print a simple table to stderr if not rank-only; or just the table if rank-only.
Phase 2: presets and commands

Extend config loader to parse [[handlers.rule]] and build a list in order.
Add built-in “csv_head” preset using csv crate (feature-gated if you want).
Add “command” handler that:
Interpolates {path}, {repo_root}, {target_path}, {relpath}, etc.
Enforces timeout and captures stdout.
Note: consider piping file content to stdin for handlers that prefer stdin over file path; make this configurable (stdin = "path" | "content" | "none").
Phase 3: LLM hooks (DSPy integration via external scripts)

Provide example scripts (not dependencies) in an examples/llm-hooks/ folder with README:
pdf_llm_gemini.py, pdf_llm_openai.py, summarizer.py
Show how to register in config using [[handlers.rule]] with command, and environment variables for API keys.
Optionally add a “llm” feature that just adds helper code (timeout defaults, logs, maybe a convenience preset that expands to the relevant command array), but keep Python entirely external to Rust.
Phase 4: optional enhancements

Add [limits] to config for max sizes.
Add postprocessors to operate on the combined content (e.g., de-duplicate repeated blocks). Keep this opt-in and experimental.
Answers to your specific ideas

A) Top-N files by token count

Yes, straightforward once processing yields per-file stats. Provide CLI flags:
--top N (default 10)
--rank-by tokens|bytes|words
--rank-only
--json for machine-readable output (optional)
UX: If used with normal run, still write content to stdout/file/clipboard and print the ranking table to stderr.
B) Presets for certain file types (CSV-head) and general hooks

An ordered [handlers] registry is the right model.
Support both built-ins (csv_head, pdf_extract, text, skip) and external commands.
External commands are the general design pattern that supports DSPy, custom preprocessors, etc.
Provide placeholder interpolation and timeouts. Carefully document security impact.
Keep cross-platform by allowing either shell or direct command exec (command_shell = true/false).
C) LLM integration via DSPy as PDF parsing backend

Do it via external commands, with example Python scripts. This keeps Rust clean and optional.
You can ship a “preset” name that just expands to an external command if the llm feature is enabled and a script path is configured; otherwise, warn with helpful instructions.
Add optional caching to avoid recomputation across runs.
Small doc tweaks

Mention negation patterns in ignore files (“!pattern”) are supported.
Clarify that in Git mode, headers are repo-root relative even if scoping to a subdir; in non-Git, they’re target-path relative.
Document the new [handlers] config and examples (CSV head and external command).
Document new ranking CLI flags.
Overall assessment

The codebase is already clean and well-tested. Logging, configs, and tests are solid.
The biggest correctness issues are small (token header slicing) and performance (walkdir pruning).
The feature ideas map naturally onto a small internal refactor (processed items + registry) and a config extension for handlers.
LLM integration is best done via external command hooks with examples; don’t tie Rust to Python runtime.
If you want, I can draft the handler registry types and a csv_head builtin in a follow-up patch to show exactly how it plugs into your current processing.rs.
