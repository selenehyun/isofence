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
    pub config_path: Option<PathBuf>,
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
    pub framework: Option<String>,

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
            config_path: None,
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
                    config.config_path = Some(config_path);
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
        if let Some(ref fw) = file.framework {
            match fw.to_lowercase().as_str() {
                "vitest" => self.framework = TestFramework::Vitest,
                "jest" => self.framework = TestFramework::Jest,
                _ => {} // ignore unknown values
            }
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

/// Detect test framework using a multi-strategy approach:
/// 1. project_root/package.json (deps + scoped + scripts)
/// 2. Parent directory package.json traversal (monorepo)
/// 3. Config file existence (vitest.config.ts, jest.config.ts, etc.)
fn detect_framework(project_root: &Path) -> TestFramework {
    // Strategy 1: project root package.json
    let pkg_path = project_root.join("package.json");
    if let Some(fw) = detect_from_package_json(&pkg_path) {
        return fw;
    }

    // Strategy 2: parent directory traversal (monorepo support, up to 5 levels)
    if let Some(fw) = detect_from_parent_package_jsons(project_root) {
        return fw;
    }

    // Strategy 3: config file existence
    if let Some(fw) = detect_from_config_files(project_root) {
        return fw;
    }

    TestFramework::Unknown
}

/// Detect framework from a single package.json file.
/// Vitest-first: checks all vitest signals before jest signals.
fn detect_from_package_json(pkg_path: &Path) -> Option<TestFramework> {
    let content = std::fs::read_to_string(pkg_path).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;

    // Collect all dependency keys
    let has_dep = |name: &str| -> bool {
        ["devDependencies", "dependencies"]
            .iter()
            .any(|key| pkg.get(key).and_then(|v| v.as_object()).is_some_and(|deps| deps.contains_key(name)))
    };

    let has_scoped = |prefix: &str| -> bool {
        ["devDependencies", "dependencies"].iter().any(|key| {
            pkg.get(key)
                .and_then(|v| v.as_object())
                .is_some_and(|deps| deps.keys().any(|k| k.starts_with(prefix)))
        })
    };

    let scripts_contain = |needle: &str| -> bool {
        pkg.get("scripts")
            .and_then(|v| v.as_object())
            .is_some_and(|scripts| {
                scripts.values().any(|v| v.as_str().is_some_and(|s| s.contains(needle)))
            })
    };

    // Vitest checks first (all of them before any jest check)
    if has_dep("vitest") {
        return Some(TestFramework::Vitest);
    }
    if has_scoped("@vitest/") {
        return Some(TestFramework::Vitest);
    }
    if scripts_contain("vitest") {
        return Some(TestFramework::Vitest);
    }

    // Jest checks
    if has_dep("jest") {
        return Some(TestFramework::Jest);
    }
    if has_scoped("@jest/") {
        return Some(TestFramework::Jest);
    }
    if scripts_contain("jest") {
        return Some(TestFramework::Jest);
    }

    None
}

/// Walk up parent directories (max 5 levels) looking for package.json with framework hints.
fn detect_from_parent_package_jsons(project_root: &Path) -> Option<TestFramework> {
    let mut current = project_root.parent()?;
    for _ in 0..5 {
        let pkg_path = current.join("package.json");
        if let Some(fw) = detect_from_package_json(&pkg_path) {
            return Some(fw);
        }
        current = current.parent()?;
    }
    None
}

/// Detect framework from config file existence.
fn detect_from_config_files(project_root: &Path) -> Option<TestFramework> {
    // Vitest config files
    let vitest_configs = [
        "vitest.config.ts",
        "vitest.config.js",
        "vitest.config.mts",
        "vitest.config.mjs",
        "vitest.workspace.ts",
        "vitest.workspace.js",
    ];
    for name in vitest_configs {
        if project_root.join(name).exists() {
            return Some(TestFramework::Vitest);
        }
    }

    // Jest config files
    let jest_configs = [
        "jest.config.ts",
        "jest.config.js",
        "jest.config.mjs",
        "jest.config.cjs",
    ];
    for name in jest_configs {
        if project_root.join(name).exists() {
            return Some(TestFramework::Jest);
        }
    }

    None
}

/// Infer framework by scanning test file contents for vi.mock/vi.fn vs jest.mock/jest.fn.
/// Vitest-first: if any vi pattern is found, returns Vitest.
pub fn infer_framework_from_test_files(files: &[PathBuf], max_files: usize) -> Option<TestFramework> {
    let mut vi_found = false;
    let mut jest_found = false;

    for file in files.iter().take(max_files) {
        if let Ok(content) = std::fs::read_to_string(file) {
            if content.contains("vi.mock(") || content.contains("vi.fn(") {
                vi_found = true;
            }
            if content.contains("jest.mock(") || content.contains("jest.fn(") {
                jest_found = true;
            }
            // Early exit: vitest always wins
            if vi_found {
                return Some(TestFramework::Vitest);
            }
        }
    }

    if jest_found {
        return Some(TestFramework::Jest);
    }

    None
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

    // ---- Framework detection tests ----

    use std::fs;
    use tempfile::TempDir;

    fn write_package_json(dir: &Path, content: &str) {
        fs::write(dir.join("package.json"), content).unwrap();
    }

    #[test]
    fn detect_from_package_json_vitest_dep() {
        let tmp = TempDir::new().unwrap();
        write_package_json(tmp.path(), r#"{"devDependencies":{"vitest":"^1.0.0"}}"#);
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_package_json_jest_dep() {
        let tmp = TempDir::new().unwrap();
        write_package_json(tmp.path(), r#"{"devDependencies":{"jest":"^29.0.0"}}"#);
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Jest));
    }

    #[test]
    fn detect_from_package_json_vitest_scoped() {
        let tmp = TempDir::new().unwrap();
        write_package_json(
            tmp.path(),
            r#"{"devDependencies":{"@vitest/coverage-v8":"^1.0.0"}}"#,
        );
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_package_json_jest_scoped() {
        let tmp = TempDir::new().unwrap();
        write_package_json(
            tmp.path(),
            r#"{"devDependencies":{"@jest/globals":"^29.0.0"}}"#,
        );
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Jest));
    }

    #[test]
    fn detect_from_package_json_vitest_script() {
        let tmp = TempDir::new().unwrap();
        write_package_json(tmp.path(), r#"{"scripts":{"test":"vitest run"}}"#);
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_package_json_jest_script() {
        let tmp = TempDir::new().unwrap();
        write_package_json(tmp.path(), r#"{"scripts":{"test":"jest --coverage"}}"#);
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Jest));
    }

    #[test]
    fn detect_from_package_json_vitest_wins_when_both_present() {
        let tmp = TempDir::new().unwrap();
        write_package_json(
            tmp.path(),
            r#"{"devDependencies":{"vitest":"^1.0.0","jest":"^29.0.0"}}"#,
        );
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_package_json_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        write_package_json(tmp.path(), r#"{"name":"my-app"}"#);
        let result = detect_from_package_json(&tmp.path().join("package.json"));
        assert_eq!(result, None);
    }

    #[test]
    fn detect_from_config_files_vitest() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("vitest.config.ts"), "").unwrap();
        let result = detect_from_config_files(tmp.path());
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_config_files_jest() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("jest.config.ts"), "").unwrap();
        let result = detect_from_config_files(tmp.path());
        assert_eq!(result, Some(TestFramework::Jest));
    }

    #[test]
    fn detect_from_config_files_vitest_workspace() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("vitest.workspace.ts"), "").unwrap();
        let result = detect_from_config_files(tmp.path());
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_from_parent_package_jsons_monorepo() {
        let tmp = TempDir::new().unwrap();
        // Root has vitest
        write_package_json(tmp.path(), r#"{"devDependencies":{"vitest":"^1.0.0"}}"#);
        // Sub-project has no package.json with framework
        let sub = tmp.path().join("packages").join("app");
        fs::create_dir_all(&sub).unwrap();
        write_package_json(&sub, r#"{"name":"app"}"#);

        let result = detect_from_parent_package_jsons(&sub);
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn detect_framework_full_fallthrough() {
        // No package.json, no config files → Unknown
        let tmp = TempDir::new().unwrap();
        let result = detect_framework(tmp.path());
        assert_eq!(result, TestFramework::Unknown);
    }

    #[test]
    fn infer_framework_from_test_files_vitest() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("app.test.ts");
        fs::write(&test_file, "vi.mock('./module');\nvi.fn();").unwrap();
        let result = infer_framework_from_test_files(&[test_file], 10);
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn infer_framework_from_test_files_jest() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("app.test.ts");
        fs::write(&test_file, "jest.mock('./module');\njest.fn();").unwrap();
        let result = infer_framework_from_test_files(&[test_file], 10);
        assert_eq!(result, Some(TestFramework::Jest));
    }

    #[test]
    fn infer_framework_from_test_files_vitest_wins_mixed() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("a.test.ts");
        let f2 = tmp.path().join("b.test.ts");
        fs::write(&f1, "jest.mock('./x');").unwrap();
        fs::write(&f2, "vi.mock('./y');").unwrap();
        let result = infer_framework_from_test_files(&[f1, f2], 10);
        assert_eq!(result, Some(TestFramework::Vitest));
    }

    #[test]
    fn infer_framework_from_test_files_none() {
        let tmp = TempDir::new().unwrap();
        let test_file = tmp.path().join("app.test.ts");
        fs::write(&test_file, "describe('test', () => {});").unwrap();
        let result = infer_framework_from_test_files(&[test_file], 10);
        assert_eq!(result, None);
    }

    #[test]
    fn config_file_framework_override_vitest() {
        let tmp = TempDir::new().unwrap();
        // package.json says jest
        write_package_json(tmp.path(), r#"{"devDependencies":{"jest":"^29.0.0"}}"#);
        // isofence.json overrides to vitest
        fs::write(
            tmp.path().join("isofence.json"),
            r#"{"framework":"vitest"}"#,
        )
        .unwrap();

        let config = Config::load(tmp.path().to_path_buf());
        assert_eq!(config.framework, TestFramework::Vitest);
    }

    #[test]
    fn config_file_framework_override_jest() {
        let tmp = TempDir::new().unwrap();
        // vitest.config.ts exists
        fs::write(tmp.path().join("vitest.config.ts"), "").unwrap();
        // isofence.json overrides to jest
        fs::write(
            tmp.path().join("isofence.json"),
            r#"{"framework":"jest"}"#,
        )
        .unwrap();

        let config = Config::load(tmp.path().to_path_buf());
        assert_eq!(config.framework, TestFramework::Jest);
    }
}
