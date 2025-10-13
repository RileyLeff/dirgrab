Great progress. These bugfixes address almost everything we discussed and they’re implemented cleanly. A quick pass-by-pass with any remaining tweaks to ship 0.3.0 confidently:

What you fixed well

tokens_exclude_tree: fixed

strip_tree_section now slices after the “FILE CONTENTS” header and falls back to the full output if the header isn’t present. This resolves the under/over-counting bug.
Git repo detection: improved

Canonicalize now reports the real path in IoError. Good.
“dubious ownership” is handled with a warn and remediation hint, then fallback to non-git. Perfect UX.
Walkdir pruning: fixed

You now prune ignored directories early via skip_current_dir. This will reduce IO churn massively on big trees.
Pattern normalization: fixed

normalize_glob is used in both Git pathspecs and walkdir ignore builder, plus in the config loader. This resolves Windows backslash patterns.
Docs duplication: fixed

You removed the second include_str on grab_contents. Good.
Release workflow: fixed and simplified

Null-delimited ls-files piped to tar. Good.
Heredoc for GITHUB_OUTPUT is correct, and permissions are set.
Path rendering normalization in headers

normalized_path for display ensures forward slashes for consistency. Nice touch.
One important regression to fix before release

dirgrab-lib readme and include_str paths will break packaging/docs
Current state:
dirgrab-lib/Cargo.toml: readme = "../README.md"
dirgrab-lib/src/lib.rs: include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../README.md"))
Problem: Cargo package rules require the readme to be inside the crate’s package directory. Including a file outside the crate root will fail on crates.io/docs.rs builds.
Fix (recommended):
Put a README.md inside dirgrab-lib (a copy of the workspace README).
Set dirgrab-lib/Cargo.toml: readme = "README.md"
In lib.rs: use include_str!("README.md")
If you removed the pre-commit sync hook, either restore a simple copy step (pre-commit or a tiny “sync-readmes” script/Makefile target), or commit the crate README by hand.
Do the same for the binary crate if you want it to have a readme on crates.io (optional); otherwise omit the readme key entirely there.
Smaller polish items

list_files_walkdir: your manual while + skip_current_dir is fine. If you ever want, filter_entry is a more declarative alternative, but no change needed.

Tests: consider adding one for a Windows-style exclude pattern (e.g., push("subdir\")). Your normalization should make it pass on all platforms.

Release source archive: using git ls-files alone is fine since LICENSE/README are tracked. If they weren’t, you’d need to append them explicitly, but your repo tracks them.

Workspace dependency version pin

You can simplify dirgrab/Cargo.toml by dropping version = "0.3.0" on the path dependency and just using dirgrab-lib = { path = "../dirgrab-lib" }. Not required, just less churn.
What changed per our earlier feedback (confirming)

walkdir prune: yes
normalize exclude patterns: yes
dubious ownership: warn + hint: yes
tokens exclude tree fix: yes
docs duplication: yes
release URL issue: resolved by removing the custom URL section and relying on the action to upload artifacts + body from CHANGELOG. Good.
What’s left for 0.3.0 readiness

Fix the README packaging issue (most critical).
Optional: restore a readme sync mechanism (pre-commit hook or a CI check) to keep crate readmes current.
Cut the tag and ensure docs.rs and crates.io build succeed.
If you want a one-liner to sync readmes pre-release without pre-commit:

Add a Makefile target:
make sync-readmes:
cp README.md dirgrab-lib/README.md
cp README.md dirgrab/README.md
Run it before tagging.
Once you fix the README packaging, I’d ship 0.3.0. The new features can layer on cleanly after this.