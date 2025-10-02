use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};
use directories::BaseDirs;
use log::{debug, warn};
use serde::Deserialize;

use dirgrab_lib::GrabConfig;

use crate::Cli;

#[derive(Debug, Clone)]
pub struct RunSettings {
    pub grab_config: GrabConfig,
    pub stats: StatsSettings,
}

#[derive(Debug, Clone)]
pub struct StatsSettings {
    pub enabled: bool,
    pub token_ratio: f64,
    pub exclude_tree: bool,
    pub exclude_headers: bool,
}

const DEFAULT_TOKEN_RATIO: f64 = 3.6;

pub fn build_run_settings(cli: &Cli, target_path: &Path) -> Result<RunSettings> {
    let mut flags = Flags::default();
    let mut stats_acc = StatsAccum::default();
    let mut patterns = PatternAccumulator::default();

    if !cli.no_config {
        if let Some(base_dirs) = BaseDirs::new() {
            let config_dir = base_dirs.config_dir().join("dirgrab");
            let global_config_path = config_dir.join("config.toml");
            apply_config_file(
                &global_config_path,
                &mut flags,
                &mut stats_acc,
                &mut patterns,
            )?;

            let global_ignore_path = config_dir.join("ignore");
            apply_ignore_file(&global_ignore_path, &mut patterns)?;
        } else {
            debug!("No base directories available; skipping global config search");
        }

        let local_config_path = target_path.join(".dirgrab.toml");
        apply_config_file(
            &local_config_path,
            &mut flags,
            &mut stats_acc,
            &mut patterns,
        )?;

        let local_ignore_path = target_path.join(".dirgrabignore");
        apply_ignore_file(&local_ignore_path, &mut patterns)?;

        if let Some(explicit_path) = cli.config_path.as_ref() {
            apply_config_file(explicit_path, &mut flags, &mut stats_acc, &mut patterns)?;
        }
    } else if let Some(explicit_path) = cli.config_path.as_ref() {
        debug!(
            "--no-config specified; skipping explicitly requested config file {:?}",
            explicit_path
        );
    }

    // CLI overrides (highest precedence)
    if cli.no_headers {
        flags.add_headers = false;
    }
    if cli.no_tree {
        flags.include_tree = false;
    }
    if cli.no_pdf {
        flags.convert_pdf = false;
    }
    if cli.include_default_output {
        flags.include_default_output = true;
    }
    if cli.no_git {
        flags.no_git = true;
    }
    if cli.all_repo {
        flags.all_repo = true;
    }
    if cli.tracked_only {
        flags.include_untracked = false;
    }
    if cli.include_untracked_flag {
        flags.include_untracked = true;
    }

    // CLI excludes
    for pattern in &cli.exclude_patterns {
        patterns.push(pattern);
    }

    // Auto-exclude active output file basename unless explicitly included
    if let Some(ref output_path) = cli.output {
        if let Some(name) = output_path.file_name().and_then(|n| n.to_str()) {
            let should_skip_default =
                name.eq_ignore_ascii_case("dirgrab.txt") && flags.include_default_output;
            if !should_skip_default {
                patterns.push(name);
            }
        }
    }

    // Stats merging
    if cli.print_stats {
        stats_acc.enabled = Some(true);
    }
    if let Some(ratio) = cli.token_ratio {
        if ratio <= 0.0 {
            bail!("--token-ratio must be greater than 0");
        }
        stats_acc.token_ratio = Some(ratio);
    }
    if cli.tokens_exclude_tree {
        stats_acc.exclude_tree = Some(true);
    }
    if cli.tokens_exclude_headers {
        stats_acc.exclude_headers = Some(true);
    }

    let stats = StatsSettings {
        enabled: stats_acc.enabled.unwrap_or(false),
        token_ratio: stats_acc.token_ratio.unwrap_or(DEFAULT_TOKEN_RATIO),
        exclude_tree: stats_acc.exclude_tree.unwrap_or(false),
        exclude_headers: stats_acc.exclude_headers.unwrap_or(false),
    };

    let grab_config = GrabConfig {
        target_path: target_path.to_path_buf(),
        add_headers: flags.add_headers,
        exclude_patterns: patterns.into_vec(),
        include_untracked: flags.include_untracked,
        include_default_output: flags.include_default_output,
        no_git: flags.no_git,
        include_tree: flags.include_tree,
        convert_pdf: flags.convert_pdf,
        all_repo: flags.all_repo,
    };

    Ok(RunSettings { grab_config, stats })
}

#[derive(Debug)]
struct Flags {
    add_headers: bool,
    include_tree: bool,
    convert_pdf: bool,
    include_default_output: bool,
    include_untracked: bool,
    no_git: bool,
    all_repo: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            add_headers: true,
            include_tree: true,
            convert_pdf: true,
            include_default_output: false,
            include_untracked: true,
            no_git: false,
            all_repo: false,
        }
    }
}

#[derive(Debug, Default)]
struct StatsAccum {
    enabled: Option<bool>,
    token_ratio: Option<f64>,
    exclude_tree: Option<bool>,
    exclude_headers: Option<bool>,
}

#[derive(Debug, Default)]
struct PatternAccumulator {
    patterns: Vec<String>,
    seen: HashSet<String>,
}

impl PatternAccumulator {
    fn push<S: AsRef<str>>(&mut self, pattern: S) {
        let candidate = pattern.as_ref().trim();
        if candidate.is_empty() {
            return;
        }
        let owned = candidate.to_string();
        if self.seen.insert(owned.clone()) {
            debug!("Adding exclude pattern: {}", owned);
            self.patterns.push(owned);
        } else {
            debug!("Skipping duplicate exclude pattern: {}", owned);
        }
    }

    fn merge<I>(&mut self, iter: I)
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        for item in iter {
            self.push(item);
        }
    }

    fn into_vec(self) -> Vec<String> {
        self.patterns
    }
}

fn apply_config_file(
    path: &Path,
    flags: &mut Flags,
    stats: &mut StatsAccum,
    patterns: &mut PatternAccumulator,
) -> Result<()> {
    if !path.exists() {
        debug!("Config file {:?} not found; skipping", path);
        return Ok(());
    }

    debug!("Loading config from {:?}", path);
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file {:?}", path))?;

    let parsed: FileConfig = toml::from_str(&contents)
        .with_context(|| format!("Failed to parse config file {:?}", path))?;

    if let Some(dirgrab_section) = parsed.dirgrab {
        apply_dirgrab_section(dirgrab_section, flags, patterns);
    }
    if let Some(stats_section) = parsed.stats {
        apply_stats_section(stats_section, stats)?;
    }

    Ok(())
}

fn apply_dirgrab_section(
    section: DirgrabSection,
    flags: &mut Flags,
    patterns: &mut PatternAccumulator,
) {
    if let Some(values) = section.exclude {
        patterns.merge(values);
    }
    if let Some(value) = section.include_untracked {
        flags.include_untracked = value;
    }
    if let Some(value) = section.include_tree {
        flags.include_tree = value;
    }
    if let Some(value) = section.add_headers {
        flags.add_headers = value;
    }
    if let Some(value) = section.convert_pdf {
        flags.convert_pdf = value;
    }
    if let Some(value) = section.include_default_output {
        flags.include_default_output = value;
    }
    if let Some(value) = section.no_git {
        flags.no_git = value;
    }
    if let Some(value) = section.tracked_only {
        flags.include_untracked = !value;
    }
    if let Some(value) = section.all_repo {
        flags.all_repo = value;
    }
}

fn apply_stats_section(section: StatsSection, stats: &mut StatsAccum) -> Result<()> {
    if let Some(enabled) = section.enabled {
        stats.enabled = Some(enabled);
    }
    if let Some(ratio) = section.token_ratio {
        if ratio <= 0.0 {
            bail!("stats.token_ratio must be greater than 0");
        }
        stats.token_ratio = Some(ratio);
    }
    if let Some(tokens_exclude) = section.tokens_exclude {
        let mut exclude_tree = false;
        let mut exclude_headers = false;
        for raw in tokens_exclude {
            match raw.trim() {
                "tree" => exclude_tree = true,
                "headers" => exclude_headers = true,
                other => warn!(
                    "Unknown tokens_exclude entry '{}' in stats config; ignoring",
                    other
                ),
            }
        }
        stats.exclude_tree = Some(exclude_tree);
        stats.exclude_headers = Some(exclude_headers);
    }

    Ok(())
}

fn apply_ignore_file(path: &Path, patterns: &mut PatternAccumulator) -> Result<()> {
    if !path.exists() {
        debug!("Ignore file {:?} not found; skipping", path);
        return Ok(());
    }

    debug!("Loading ignore patterns from {:?}", path);
    let contents = fs::read_to_string(path)
        .with_context(|| format!("Failed to read ignore file {:?}", path))?;

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        patterns.push(trimmed);
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    #[serde(default)]
    dirgrab: Option<DirgrabSection>,
    #[serde(default)]
    stats: Option<StatsSection>,
}

#[derive(Debug, Deserialize)]
struct DirgrabSection {
    exclude: Option<Vec<String>>,
    include_untracked: Option<bool>,
    include_tree: Option<bool>,
    add_headers: Option<bool>,
    convert_pdf: Option<bool>,
    include_default_output: Option<bool>,
    no_git: Option<bool>,
    tracked_only: Option<bool>,
    all_repo: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct StatsSection {
    enabled: Option<bool>,
    token_ratio: Option<f64>,
    tokens_exclude: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use anyhow::Result;
    use std::collections::HashSet;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set_path(key: &'static str, path: &Path) -> Self {
            let prev = env::var(key).ok();
            env::set_var(key, path);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref value) = self.prev {
                env::set_var(self.key, value);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn isolate_env(root: &Path) -> Vec<EnvGuard> {
        let home = root.join("home");
        let xdg = root.join("xdg_config");
        let appdata = root.join("appdata");
        let local_appdata = root.join("localappdata");
        for dir in [&home, &xdg, &appdata, &local_appdata] {
            let _ = fs::create_dir_all(dir);
        }

        vec![
            EnvGuard::set_path("HOME", &home),
            EnvGuard::set_path("XDG_CONFIG_HOME", &xdg),
            EnvGuard::set_path("APPDATA", &appdata),
            EnvGuard::set_path("LOCALAPPDATA", &local_appdata),
            EnvGuard::set_path("USERPROFILE", &home),
        ]
    }

    #[test]
    fn local_config_merges_with_cli_overrides() -> Result<()> {
        let temp = tempdir()?;
        let target = temp.path().join("project");
        fs::create_dir_all(&target)?;

        let _guards = isolate_env(temp.path());

        fs::write(
            target.join(".dirgrab.toml"),
            r#"
[dirgrab]
include_tree = false
add_headers = false
tracked_only = true
exclude = ["Cargo.lock"]

[stats]
enabled = false
token_ratio = 4.4
tokens_exclude = ["tree"]
"#,
        )?;
        fs::write(target.join(".dirgrabignore"), "node_modules/\n")?;

        let mut cli = Cli::test_default();
        cli.include_untracked_flag = true;
        cli.tokens_exclude_headers = true;
        cli.token_ratio = Some(5.0);
        cli.print_stats = true;
        cli.output = Some(PathBuf::from("out.txt"));

        let settings = build_run_settings(&cli, &target)?;
        let config = settings.grab_config;
        let stats = settings.stats;

        assert!(!config.include_tree);
        assert!(!config.add_headers);
        assert!(config.include_untracked);

        let patterns: HashSet<_> = config.exclude_patterns.iter().cloned().collect();
        assert!(patterns.contains("Cargo.lock"));
        assert!(patterns.contains("node_modules/"));
        assert!(patterns.contains("out.txt"));

        assert!(stats.enabled);
        assert!(stats.exclude_tree);
        assert!(stats.exclude_headers);
        assert!((stats.token_ratio - 5.0).abs() < f64::EPSILON);

        Ok(())
    }

    #[test]
    fn no_config_skips_local_and_still_excludes_output() -> Result<()> {
        let temp = tempdir()?;
        let target = temp.path().join("project");
        fs::create_dir_all(&target)?;

        let _guards = isolate_env(temp.path());

        fs::write(
            target.join(".dirgrab.toml"),
            "[dirgrab]\ninclude_tree = false\nadd_headers = false\n",
        )?;
        fs::write(target.join(".dirgrabignore"), "test-ignore/\n")?;

        let mut cli = Cli::test_default();
        cli.no_config = true;
        cli.output = Some(PathBuf::from("out.txt"));

        let settings = build_run_settings(&cli, &target)?;
        let config = settings.grab_config;
        let stats = settings.stats;

        assert!(config.include_tree);
        assert!(config.add_headers);
        assert!(config.include_untracked);

        let patterns: HashSet<_> = config.exclude_patterns.iter().cloned().collect();
        assert!(patterns.contains("out.txt"));
        assert!(!patterns.contains("test-ignore/"));

        assert!(!stats.enabled);
        assert!((stats.token_ratio - DEFAULT_TOKEN_RATIO).abs() < f64::EPSILON);
        assert!(!stats.exclude_tree);
        assert!(!stats.exclude_headers);

        Ok(())
    }
}
