use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::engine::context::TestFramework;
use crate::rule::Severity;

/// IsoFence configuration — loaded from isofence.json (optional).
#[derive(Debug, Clone)]
pub struct Config {
    pub project_root: PathBuf,
    pub test_patterns: Vec<String>,
    pub allowlist: Vec<String>,
    pub depth: usize,
    pub mock_consensus: bool,
    pub framework: TestFramework,
    pub tsconfig_path: Option<PathBuf>,
    pub custom_rules: Vec<PathBuf>,
    pub rule_configs: HashMap<String, RuleConfig>,
    pub format: OutputFormat,
    pub fix: bool,
    pub dry_run: bool,
    pub strict: bool,
    pub quiet: bool,
}

/// Output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Console,
    Json,
}

/// Per-rule configuration.
#[derive(Debug, Clone)]
pub struct RuleConfig {
    pub severity: Severity,
    pub options: HashMap<String, serde_json::Value>,
}

/// The on-disk isofence.json schema.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigFile {
    #[serde(default)]
    pub allowlist: Vec<String>,

    #[serde(default)]
    pub depth: Option<usize>,

    #[serde(default)]
    pub mock_consensus: Option<bool>,

    #[serde(default)]
    pub rules: HashMap<String, RuleSeverityOrConfig>,

    #[serde(default)]
    pub custom_rules: Vec<String>,

    #[serde(default)]
    pub test_patterns: Vec<String>,
}

/// Rule configuration can be just a severity string or [severity, options].
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RuleSeverityOrConfig {
    Severity(String),
    WithOptions(Vec<serde_json::Value>),
}

impl Config {
    /// Create a default config for a project root.
    pub fn default_for(project_root: PathBuf) -> Self {
        let framework = detect_framework(&project_root);
        let tsconfig_path = find_tsconfig(&project_root);

        Self {
            project_root,
            test_patterns: default_test_patterns(),
            allowlist: default_allowlist(),
            depth: 1,
            mock_consensus: true,
            framework,
            tsconfig_path,
            custom_rules: Vec::new(),
            rule_configs: HashMap::new(),
            format: OutputFormat::Console,
            fix: false,
            dry_run: false,
            strict: false,
            quiet: false,
        }
    }

    /// Load config from isofence.json (if it exists), then merge with defaults.
    pub fn load(project_root: PathBuf) -> Self {
        let mut config = Self::default_for(project_root.clone());

        let config_path = project_root.join("isofence.json");
        if config_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&config_path) {
                if let Ok(file) = serde_json::from_str::<ConfigFile>(&content) {
                    config.merge_file(file);
                }
            }
        }

        config
    }

    /// Merge a config file into this config.
    fn merge_file(&mut self, file: ConfigFile) {
        if !file.allowlist.is_empty() {
            self.allowlist.extend(file.allowlist);
        }
        if let Some(depth) = file.depth {
            self.depth = depth;
        }
        if let Some(consensus) = file.mock_consensus {
            self.mock_consensus = consensus;
        }
        if !file.test_patterns.is_empty() {
            self.test_patterns = file.test_patterns;
        }
        for path in file.custom_rules {
            self.custom_rules
                .push(self.project_root.join(path));
        }

        // Parse rule configs
        for (name, value) in file.rules {
            match value {
                RuleSeverityOrConfig::Severity(s) => {
                    if let Some(sev) = parse_severity(&s) {
                        self.rule_configs.insert(
                            name,
                            RuleConfig {
                                severity: sev,
                                options: HashMap::new(),
                            },
                        );
                    }
                }
                RuleSeverityOrConfig::WithOptions(parts) => {
                    if let Some(serde_json::Value::String(s)) = parts.first() {
                        if let Some(sev) = parse_severity(s) {
                            let options = parts
                                .get(1)
                                .and_then(|v| v.as_object())
                                .map(|o| {
                                    o.iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            self.rule_configs.insert(
                                name,
                                RuleConfig { severity: sev, options },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Check if a path matches the allowlist.
    pub fn is_allowed(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();

        // Always allow node builtins and node_modules
        if path_str.starts_with("node:") || path_str.contains("node_modules") {
            return true;
        }

        for pattern in &self.allowlist {
            if glob_match(pattern, &path_str) {
                return true;
            }
        }

        false
    }
}

fn parse_severity(s: &str) -> Option<Severity> {
    match s {
        "error" => Some(Severity::Error),
        "warning" | "warn" => Some(Severity::Warning),
        "off" => Some(Severity::Off),
        _ => None,
    }
}

fn default_test_patterns() -> Vec<String> {
    vec![
        "**/*.test.ts".to_string(),
        "**/*.test.tsx".to_string(),
        "**/*.spec.ts".to_string(),
        "**/*.spec.tsx".to_string(),
        "**/__tests__/**/*.ts".to_string(),
        "**/__tests__/**/*.tsx".to_string(),
    ]
}

fn default_allowlist() -> Vec<String> {
    vec![
        "src/types/**".to_string(),
        "src/constants/**".to_string(),
    ]
}

/// Detect test framework from package.json.
fn detect_framework(project_root: &Path) -> TestFramework {
    let pkg_path = project_root.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path) {
        if let Ok(pkg) = serde_json::from_str::<serde_json::Value>(&content) {
            // Check devDependencies and dependencies
            for key in ["devDependencies", "dependencies"] {
                if let Some(deps) = pkg.get(key).and_then(|v| v.as_object()) {
                    if deps.contains_key("vitest") {
                        return TestFramework::Vitest;
                    }
                    if deps.contains_key("jest") {
                        return TestFramework::Jest;
                    }
                }
            }
        }
    }
    TestFramework::Unknown
}

/// Find tsconfig.json in the project root.
fn find_tsconfig(project_root: &Path) -> Option<PathBuf> {
    let tsconfig = project_root.join("tsconfig.json");
    if tsconfig.exists() {
        Some(tsconfig)
    } else {
        None
    }
}

/// Simple glob matching (supports `*` and `**`).
///
/// `*` matches any characters except `/`.
/// `**` matches any characters including `/` (any path depth).
/// `**/` matches zero or more directory segments.
fn glob_match(pattern: &str, path: &str) -> bool {
    // Convert glob pattern to regex
    // Order matters: replace **/ first, then **, then *
    let regex_str = pattern
        .replace('.', "\\.")
        .replace("**/", "\x00")
        .replace("**", "\x01")
        .replace('*', "[^/]*")
        .replace('\x00', "(.+/)?")
        .replace('\x01', ".*");

    regex::Regex::new(&format!("(^|/){regex_str}$"))
        .map(|r| r.is_match(path))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_match_single_star() {
        assert!(glob_match("*.ts", "foo.ts"));
        assert!(glob_match("*.ts", "src/foo.ts"));
        assert!(!glob_match("*.ts", "foo.tsx"));
    }

    #[test]
    fn glob_match_double_star_slash() {
        assert!(glob_match("src/types/**", "src/types/user.ts"));
        assert!(glob_match("src/types/**", "src/types/nested/deep.ts"));
        assert!(glob_match("src/types/**", "/abs/path/src/types/user.ts"));
        assert!(!glob_match("src/types/**", "src/other/user.ts"));
    }

    #[test]
    fn glob_match_leading_double_star() {
        assert!(glob_match("**/*.test.ts", "src/foo.test.ts"));
        assert!(glob_match("**/*.test.ts", "a/b/c/foo.test.ts"));
        assert!(glob_match("**/*.test.ts", "foo.test.ts"));
    }

    #[test]
    fn is_allowed_node_modules() {
        let config = Config::default_for(PathBuf::from("/project"));
        assert!(config.is_allowed(Path::new("/project/node_modules/lodash/index.js")));
    }

    #[test]
    fn is_allowed_node_builtins() {
        let config = Config::default_for(PathBuf::from("/project"));
        assert!(config.is_allowed(Path::new("node:fs")));
    }

    #[test]
    fn is_allowed_allowlist() {
        let config = Config::default_for(PathBuf::from("/project"));
        // Default allowlist includes src/types/** and src/constants/**
        assert!(config.is_allowed(Path::new("/project/src/types/user.ts")));
        assert!(config.is_allowed(Path::new("/project/src/constants/config.ts")));
        assert!(!config.is_allowed(Path::new("/project/src/services/api.ts")));
    }
}
