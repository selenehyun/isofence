use isofence::engine::context::{
    is_test_file_path, EdgeKind, MockDeclaration, MockKind, MutationImpact, SafeSignal,
    TestFramework,
};
use isofence::engine::graph::ModuleGraph;
use isofence::engine::parser::{
    analyze_export_mutation, collect_module_mutable_bindings, extract_exports, extract_imports,
    extract_mocks, extract_safe_signals, parse_source,
};
use oxc_allocator::Allocator;
use oxc_span::Span;
use std::path::PathBuf;

fn path(s: &str) -> PathBuf {
    PathBuf::from(s)
}

fn mock(source: &str, resolved: &str, kind: MockKind) -> MockDeclaration {
    MockDeclaration {
        source: source.to_string(),
        resolved_path: Some(PathBuf::from(resolved)),
        kind,
        span: Span::default(),
    }
}

// ---- Context Tests ----

mod context {
    use super::*;

    #[test]
    fn detects_test_file_patterns() {
        assert!(is_test_file_path(&path("src/foo.test.ts")));
        assert!(is_test_file_path(&path("src/foo.spec.ts")));
        assert!(is_test_file_path(&path("src/foo.test.tsx")));
        assert!(is_test_file_path(&path("src/foo.spec.tsx")));
        assert!(is_test_file_path(&path("src/__tests__/foo.ts")));
        assert!(is_test_file_path(&path("src/foo.test.js")));
        assert!(is_test_file_path(&path("src/foo.spec.jsx")));
        // Test infrastructure files
        assert!(is_test_file_path(&path("src/data.mock.ts")));
        assert!(is_test_file_path(&path("src/data.mock.tsx")));
        assert!(is_test_file_path(&path("src/__mocks__/foo.ts")));
        assert!(is_test_file_path(&path("src/__fixtures__/data.ts")));
    }

    #[test]
    fn rejects_non_test_files() {
        assert!(!is_test_file_path(&path("src/foo.ts")));
        assert!(!is_test_file_path(&path("src/utils.tsx")));
        assert!(!is_test_file_path(&path("src/index.ts")));
    }

    #[test]
    fn framework_mock_fn_name() {
        assert_eq!(TestFramework::Vitest.mock_fn_name(), "vi.mock");
        assert_eq!(TestFramework::Jest.mock_fn_name(), "jest.mock");
    }
}

// ---- Parser Tests ----

mod parser {
    use super::*;

    #[test]
    fn parses_typescript() {
        let allocator = Allocator::default();
        let source = "const x: number = 42;\nexport function add(a: number, b: number): number { return a + b; }";
        let result = parse_source(&allocator, source, &path("test.ts"));
        assert!(!result.panicked);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn extracts_imports() {
        let allocator = Allocator::default();
        let source = r#"
import { foo } from './foo';
import type { Bar } from './bar';
import './setup';
import * as utils from './utils';
"#;
        let result = parse_source(&allocator, source, &path("test.ts"));
        let imports = extract_imports(&result.program);

        assert_eq!(imports.len(), 4);
        assert_eq!(imports[0].source, "./foo");
        assert!(!imports[0].is_type_only);
        assert!(!imports[0].is_side_effect);

        assert_eq!(imports[1].source, "./bar");
        assert!(imports[1].is_type_only);

        assert_eq!(imports[2].source, "./setup");
        assert!(imports[2].is_side_effect);

        assert_eq!(imports[3].source, "./utils");
        assert!(!imports[3].is_side_effect);
    }

    #[test]
    fn extracts_reexports() {
        let allocator = Allocator::default();
        let source = r#"
export { foo } from './foo';
export * from './bar';
export type { Baz } from './baz';
"#;
        let result = parse_source(&allocator, source, &path("index.ts"));
        let imports = extract_imports(&result.program);

        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].source, "./foo");
        assert_eq!(imports[1].source, "./bar");
        assert_eq!(imports[2].source, "./baz");
        assert!(imports[2].is_type_only);
    }

    #[test]
    fn extracts_vitest_mocks() {
        let allocator = Allocator::default();
        let source = r#"
vi.mock('./database');
vi.mock('./logger', () => ({ log: vi.fn() }));
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let mocks = extract_mocks(&result.program, TestFramework::Vitest);

        assert_eq!(mocks.len(), 2);
        assert_eq!(mocks[0].source, "./database");
        assert_eq!(mocks[0].kind, MockKind::Full);
        assert_eq!(mocks[1].source, "./logger");
        assert_eq!(mocks[1].kind, MockKind::Full);
    }

    #[test]
    fn extracts_jest_mocks() {
        let allocator = Allocator::default();
        let source = r#"
jest.mock('./database');
jest.mock('./logger', () => ({ log: jest.fn() }));
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let mocks = extract_mocks(&result.program, TestFramework::Jest);

        assert_eq!(mocks.len(), 2);
        assert_eq!(mocks[0].source, "./database");
        assert_eq!(mocks[1].source, "./logger");
    }

    #[test]
    fn detects_partial_mock() {
        let allocator = Allocator::default();
        let source = r#"
vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal();
  return { ...actual, fetchData: vi.fn() };
});
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let mocks = extract_mocks(&result.program, TestFramework::Vitest);

        assert_eq!(mocks.len(), 1);
        assert_eq!(mocks[0].kind, MockKind::Partial);
    }

    #[test]
    fn full_mock_with_factory() {
        let allocator = Allocator::default();
        let source = r#"
vi.mock('./logger', () => ({ log: vi.fn(), warn: vi.fn() }));
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let mocks = extract_mocks(&result.program, TestFramework::Vitest);

        assert_eq!(mocks.len(), 1);
        assert_eq!(mocks[0].kind, MockKind::Full);
    }

    #[test]
    fn safe_signals_after_each_detected() {
        let allocator = Allocator::default();
        let source = r#"
afterEach(() => {
    vi.restoreAllMocks();
});
test('a', () => {});
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let signals = extract_safe_signals(&result.program);

        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0], SafeSignal::RestoreAllMocks);
    }

    #[test]
    fn safe_signals_ignores_non_vi_calls() {
        let allocator = Allocator::default();
        let source = r#"
beforeEach(() => {
    myCache.clear();
    someStore.reset();
});
test('a', () => {});
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let signals = extract_safe_signals(&result.program);

        assert!(
            signals.is_empty(),
            "Non-vi/jest calls like myCache.clear() should not be detected as safe signals, got: {:?}",
            signals
        );
    }

    #[test]
    fn safe_signals_clear_all_mocks_detected() {
        let allocator = Allocator::default();
        let source = r#"
beforeEach(() => {
    vi.clearAllMocks();
});
test('a', () => {});
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let signals = extract_safe_signals(&result.program);

        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0], SafeSignal::ClearAllMocks);
    }

    #[test]
    fn safe_signals_jest_prefix_detected() {
        let allocator = Allocator::default();
        let source = r#"
afterEach(() => {
    jest.clearAllMocks();
    jest.resetAllMocks();
});
test('a', () => {});
"#;
        let result = parse_source(&allocator, source, &path("test.test.ts"));
        let signals = extract_safe_signals(&result.program);

        assert_eq!(signals.len(), 2);
        assert!(signals.contains(&SafeSignal::ClearAllMocks));
        assert!(signals.contains(&SafeSignal::ResetAllMocks));
    }
}

// ---- Graph Tests ----

mod graph {
    use super::*;

    #[test]
    fn effective_subgraph_basic() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);
        graph.add_node(path("/c.ts"), false);

        graph.add_edge(path("/test.ts"), path("/a.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/test.ts"), path("/b.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/a.ts"), path("/c.ts"), EdgeKind::StaticImport, false);

        let subgraph = graph.effective_subgraph(&path("/test.ts"), &[]);
        assert!(subgraph.contains(&path("/test.ts")));
        assert!(subgraph.contains(&path("/a.ts")));
        assert!(subgraph.contains(&path("/b.ts")));
        assert!(subgraph.contains(&path("/c.ts")));
    }

    #[test]
    fn effective_subgraph_with_full_mock() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);
        graph.add_node(path("/c.ts"), false);

        graph.add_edge(path("/test.ts"), path("/a.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/a.ts"), path("/b.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/b.ts"), path("/c.ts"), EdgeKind::StaticImport, false);

        let mocks = vec![mock("./a", "/a.ts", MockKind::Full)];
        let subgraph = graph.effective_subgraph(&path("/test.ts"), &mocks);

        assert!(subgraph.contains(&path("/test.ts")));
        assert!(!subgraph.contains(&path("/a.ts")));
        assert!(!subgraph.contains(&path("/b.ts")));
        assert!(!subgraph.contains(&path("/c.ts")));
    }

    #[test]
    fn effective_subgraph_with_partial_mock() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);

        graph.add_edge(path("/test.ts"), path("/a.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/a.ts"), path("/b.ts"), EdgeKind::StaticImport, false);

        let mocks = vec![mock("./a", "/a.ts", MockKind::Partial)];
        let subgraph = graph.effective_subgraph(&path("/test.ts"), &mocks);

        assert!(subgraph.contains(&path("/a.ts")));
        assert!(subgraph.contains(&path("/b.ts")));
    }

    #[test]
    fn effective_subgraph_skips_type_only() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/types.ts"), false);

        graph.add_edge(path("/test.ts"), path("/types.ts"), EdgeKind::StaticImport, true);

        let subgraph = graph.effective_subgraph(&path("/test.ts"), &[]);
        assert!(!subgraph.contains(&path("/types.ts")));
    }

    #[test]
    fn shortest_path_basic() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);

        graph.add_edge(path("/test.ts"), path("/a.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/a.ts"), path("/b.ts"), EdgeKind::StaticImport, false);

        let sp = graph.shortest_path(&path("/test.ts"), &path("/b.ts")).unwrap();
        assert_eq!(sp, vec![path("/test.ts"), path("/a.ts"), path("/b.ts")]);
    }

    #[test]
    fn handles_circular_references() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);

        graph.add_edge(path("/a.ts"), path("/b.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/b.ts"), path("/a.ts"), EdgeKind::StaticImport, false);

        let subgraph = graph.effective_subgraph(&path("/a.ts"), &[]);
        assert!(subgraph.contains(&path("/a.ts")));
        assert!(subgraph.contains(&path("/b.ts")));
    }

    #[test]
    fn mock_cuts_transitive_chain() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/service.ts"), false);
        graph.add_node(path("/db.ts"), false);
        graph.add_node(path("/cache.ts"), false);

        // test → service → db → cache
        graph.add_edge(path("/test.ts"), path("/service.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/service.ts"), path("/db.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/db.ts"), path("/cache.ts"), EdgeKind::StaticImport, false);

        // Mock service — cuts entire chain
        let mocks = vec![mock("./service", "/service.ts", MockKind::Full)];
        let subgraph = graph.effective_subgraph(&path("/test.ts"), &mocks);

        assert!(!subgraph.contains(&path("/service.ts")));
        assert!(!subgraph.contains(&path("/db.ts")));
        assert!(!subgraph.contains(&path("/cache.ts")));
    }

    #[test]
    fn diamond_dependency() {
        let mut graph = ModuleGraph::new();
        graph.add_node(path("/test.ts"), true);
        graph.add_node(path("/a.ts"), false);
        graph.add_node(path("/b.ts"), false);
        graph.add_node(path("/shared.ts"), false);

        // test → a → shared
        // test → b → shared
        graph.add_edge(path("/test.ts"), path("/a.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/test.ts"), path("/b.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/a.ts"), path("/shared.ts"), EdgeKind::StaticImport, false);
        graph.add_edge(path("/b.ts"), path("/shared.ts"), EdgeKind::StaticImport, false);

        // Mock /a.ts — /shared.ts still reachable via /b.ts
        let mocks = vec![mock("./a", "/a.ts", MockKind::Full)];
        let subgraph = graph.effective_subgraph(&path("/test.ts"), &mocks);

        assert!(!subgraph.contains(&path("/a.ts")));
        assert!(subgraph.contains(&path("/b.ts")));
        assert!(subgraph.contains(&path("/shared.ts")));
    }
}

// ---- Integration Tests ----

mod integration {
    use isofence::config::Config;
    use isofence::engine::context::{is_test_file_path, MutationImpact};
    use isofence::engine::Engine;
    use isofence::rule::registry::RuleRegistry;
    use isofence::rules::all_builtin_rules;
    use tempfile::tempdir;

    #[test]
    fn source_file_diagnostics_excluded_from_output() {
        let dir = tempdir().unwrap();

        // Source file with a hazard (mutable-module-var)
        let source_path = dir.path().join("counter.ts");
        std::fs::write(&source_path, "let counter = 0;\n").unwrap();

        let config = Config::load(dir.path().to_path_buf());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[], &[source_path]);

        // Source file diagnostics should be internal only — not in final output
        assert!(
            result.diagnostics.is_empty(),
            "Expected no diagnostics (source file diagnostics should be internal), got: {:?}",
            result.diagnostics.iter()
                .map(|d| format!("[{}] {}: {}", d.file_path.display(), d.rule_name, d.message))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn hazard_reachability_dedup_preserves_distinct_modules() {
        let dir = tempdir().unwrap();
        // Canonicalize to avoid macOS /var vs /private/var symlink issues
        let base = dir.path().canonicalize().unwrap();

        // Two source files with different hazards
        let source_a = base.join("module_a.ts");
        std::fs::write(&source_a, "export let stateA = 0;\n").unwrap();

        let source_b = base.join("module_b.ts");
        std::fs::write(&source_b, "export let stateB = 0;\n").unwrap();

        // Test file importing both hazardous modules
        let test_path = base.join("multi.test.ts");
        std::fs::write(
            &test_path,
            "import { stateA } from './module_a';\nimport { stateB } from './module_b';\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_path],
            &[source_a, source_b],
        );

        // Both hazardous modules should produce distinct diagnostics
        let reachability_diags: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // With dedup fix, we should see 2 separate diagnostics (one per module)
        // Before the fix, they collapsed to 1 because span was identical (0..0)
        assert!(
            reachability_diags.len() >= 2,
            "Expected at least 2 hazard-reachability diagnostics (one per hazardous module), got {}. All diags: {:?}",
            reachability_diags.len(),
            result.diagnostics.iter()
                .map(|d| format!("[{}] {}: {}", d.rule_name, d.file_path.display(), d.message))
                .collect::<Vec<_>>(),
        );

        // Verify they reference different modules
        let messages: Vec<&str> = reachability_diags.iter().map(|d| d.message.as_str()).collect();
        assert!(
            messages.iter().any(|m| m.contains("module_a")),
            "Should have diagnostic for module_a"
        );
        assert!(
            messages.iter().any(|m| m.contains("module_b")),
            "Should have diagnostic for module_b"
        );
    }

    #[test]
    fn fix_idempotency_no_duplicate_mocks() {
        let dir = tempdir().unwrap();
        // Canonicalize to avoid macOS /var vs /private/var symlink issues
        let base = dir.path().canonicalize().unwrap();

        // Source file with a hazard
        let source_path = base.join("config.ts");
        std::fs::write(&source_path, "export let dbUrl = 'postgres://localhost';\n").unwrap();

        // Test file importing the hazardous module
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import { dbUrl } from './config';\ntest('uses config', () => { expect(dbUrl).toBeDefined(); });\n",
        )
        .unwrap();

        // First run: should produce hazard-reachability diagnostic with a fix
        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());
        let engine = Engine::new(config.clone(), registry);

        let result1 = engine.run_silent(&[test_path.clone()], &[source_path.clone()]);
        let reachability1: Vec<_> = result1
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability1.is_empty(),
            "First run should produce hazard-reachability diagnostics"
        );
        assert!(
            reachability1.iter().all(|d| d.fix.is_some()),
            "Hazard-reachability diagnostics should include fixes"
        );

        // Apply fixes
        let fix_results = isofence::fix::apply_fixes(&result1.diagnostics, &config).unwrap();
        for fr in &fix_results {
            if fr.has_changes() {
                std::fs::write(&fr.file_path, &fr.fixed).unwrap();
            }
        }

        // Verify mock was inserted
        let fixed_content = std::fs::read_to_string(&test_path).unwrap();
        assert!(
            fixed_content.contains("mock(") && fixed_content.contains("config"),
            "Fix should have inserted a mock for config. Content:\n{}",
            fixed_content
        );

        // Second run: should produce NO hazard-reachability diagnostics
        let config2 = Config::load(base.clone());
        let mut registry2 = RuleRegistry::new();
        registry2.register_all(all_builtin_rules());
        let engine2 = Engine::new(config2, registry2);

        let result2 = engine2.run_silent(&[test_path.clone()], &[source_path.clone()]);
        let reachability2: Vec<_> = result2
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability2.is_empty(),
            "Second run (after fix) should have no hazard-reachability diagnostics, but got: {:?}",
            reachability2.iter()
                .map(|d| format!("{}: {}", d.file_path.display(), d.message))
                .collect::<Vec<_>>(),
        );

        // Third run: applying fixes again should not add duplicate mocks
        let config3 = Config::load(base.clone());
        let fix_results2 = isofence::fix::apply_fixes(&result2.diagnostics, &config3).unwrap();
        let changes: Vec<_> = fix_results2.iter().filter(|fr| fr.has_changes()).collect();
        assert!(
            changes.is_empty(),
            "Third run should not produce any file changes (idempotent)"
        );
    }

    #[test]
    fn transitive_hazard_not_fixable() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // db.ts — hazardous (mutable module var)
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let connection = null;\n").unwrap();

        // service.ts — clean, imports db
        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { connection } from './db';\nexport function getConn() { return connection; }\n",
        )
        .unwrap();

        // test → service (clean) → db (hazardous)
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import { getConn } from './service';\ntest('conn', () => { getConn(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[service_path, db_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // No direct hazard imports → no fixable diagnostics
        let fixable: Vec<_> = reachability.iter().filter(|d| d.fix.is_some()).collect();
        assert!(
            fixable.is_empty(),
            "Transitive hazards should NOT be fixable, got {} fixable diagnostics",
            fixable.len()
        );

        // Should have a warning-level grouped diagnostic
        let warnings: Vec<_> = reachability
            .iter()
            .filter(|d| d.severity == isofence::rule::Severity::Warning)
            .collect();
        assert!(
            !warnings.is_empty(),
            "Expected at least 1 warning for transitive hazards"
        );
        assert!(
            warnings[0].message.contains("transitive"),
            "Warning message should mention 'transitive', got: {}",
            warnings[0].message
        );
    }

    #[test]
    fn transitive_hazards_grouped_by_first_hop() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Two hazardous modules behind the same first-hop
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let dbConn = null;\n").unwrap();

        let cache_path = base.join("cache.ts");
        std::fs::write(&cache_path, "export let cacheClient = null;\n").unwrap();

        // service.ts imports both hazardous modules
        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { dbConn } from './db';\nimport { cacheClient } from './cache';\nexport function init() {}\n",
        )
        .unwrap();

        // test → service → db + cache
        let test_path = base.join("svc.test.ts");
        std::fs::write(
            &test_path,
            "import { init } from './service';\ntest('init', () => { init(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[service_path, db_path, cache_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // db and cache are both transitive via service → should be grouped into 1 diagnostic
        assert_eq!(
            reachability.len(),
            1,
            "Expected exactly 1 grouped diagnostic for 2 transitive hazards via service, got {}: {:?}",
            reachability.len(),
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );

        // Message should mention count of 2
        assert!(
            reachability[0].message.contains("2"),
            "Grouped diagnostic should mention count '2', got: {}",
            reachability[0].message
        );
    }

    #[test]
    fn mixed_direct_and_transitive() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Direct hazard: test imports hazardA directly
        let hazard_a = base.join("hazard_a.ts");
        std::fs::write(&hazard_a, "export let directState = 0;\n").unwrap();

        // Transitive hazard: test → service → hazardB
        let hazard_b = base.join("hazard_b.ts");
        std::fs::write(&hazard_b, "export let transitiveState = 0;\n").unwrap();

        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { transitiveState } from './hazard_b';\nexport function svc() { return transitiveState; }\n",
        )
        .unwrap();

        let test_path = base.join("mixed.test.ts");
        std::fs::write(
            &test_path,
            "import { directState } from './hazard_a';\nimport { svc } from './service';\ntest('mixed', () => {});\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_path],
            &[hazard_a, hazard_b, service_path],
        );

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // 1 fixable (direct hazard_a) + 1 suggestion (transitive via service)
        let fixable: Vec<_> = reachability.iter().filter(|d| d.fix.is_some()).collect();
        let suggestions: Vec<_> = reachability.iter().filter(|d| d.fix.is_none()).collect();

        assert_eq!(
            fixable.len(),
            1,
            "Expected 1 fixable diagnostic (direct hazard), got {}",
            fixable.len()
        );
        assert!(
            fixable[0].message.contains("hazard_a"),
            "Fixable diagnostic should reference hazard_a, got: {}",
            fixable[0].message
        );

        assert_eq!(
            suggestions.len(),
            1,
            "Expected 1 suggestion diagnostic (transitive via service), got {}",
            suggestions.len()
        );
        assert!(
            suggestions[0].message.contains("transitive"),
            "Suggestion should mention 'transitive', got: {}",
            suggestions[0].message
        );
        assert_eq!(
            suggestions[0].severity,
            isofence::rule::Severity::Warning,
            "Transitive diagnostic should be Warning severity"
        );
    }

    #[test]
    fn spread_config_object_not_hazardous() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Source file with spread config — should NOT be hazardous
        let config_path = base.join("config.ts");
        std::fs::write(
            &config_path,
            "import { baseConfig } from './base';\nexport const writerConfig = { ...baseConfig, synchronize: true };\n",
        )
        .unwrap();

        let base_path = base.join("base.ts");
        std::fs::write(&base_path, "export const baseConfig = { type: 'mysql', port: 3306 };\n")
            .unwrap();

        // Test file imports config
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import { writerConfig } from './config';\ntest('config', () => {});\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[config_path, base_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability.is_empty(),
            "Spread config with primitives should not produce hazard-reachability diagnostics, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn transitive_hazard_help_references_hazardous_module() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Hazardous module
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let connection = null;\n").unwrap();

        // Clean first-hop
        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { connection } from './db';\nexport function getConn() { return connection; }\n",
        )
        .unwrap();

        // Test imports service
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import { getConn } from './service';\ntest('conn', () => { getConn(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[service_path, db_path]);

        let warnings: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability" && d.severity == isofence::rule::Severity::Warning)
            .collect();

        assert!(!warnings.is_empty(), "Expected transitive hazard warnings");

        let help = warnings[0].help.as_ref().unwrap();
        // Help should reference the hazardous module path, not the first-hop
        assert!(
            help.contains("db.ts"),
            "Help should reference hazardous module 'db.ts', got: {help}"
        );
        // Should NOT reference first-hop in the old format
        assert!(
            !help.contains("Mock `service.ts` to block"),
            "Help should NOT use old first-hop format, got: {help}"
        );
    }

    #[test]
    fn transitive_hazard_help_shows_category() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        let hazard_path = base.join("state.ts");
        std::fs::write(&hazard_path, "export let counter = 0;\n").unwrap();

        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { counter } from './state';\nexport function count() { return counter; }\n",
        )
        .unwrap();

        let test_path = base.join("cat.test.ts");
        std::fs::write(
            &test_path,
            "import { count } from './service';\ntest('count', () => { count(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[service_path, hazard_path]);

        let warnings: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability" && d.severity == isofence::rule::Severity::Warning)
            .collect();

        assert!(!warnings.is_empty(), "Expected transitive hazard warnings");

        let help = warnings[0].help.as_ref().unwrap();
        assert!(
            help.contains("mutable state"),
            "Help should contain category '(mutable state)', got: {help}"
        );
    }

    #[test]
    fn transitive_hazard_help_truncates_over_3() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Create 4 hazardous modules behind a single first-hop
        for i in 1..=4 {
            let name = format!("hazard{i}.ts");
            std::fs::write(base.join(&name), format!("export let state{i} = 0;\n")).unwrap();
        }

        // Service imports all 4
        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { state1 } from './hazard1';\n\
             import { state2 } from './hazard2';\n\
             import { state3 } from './hazard3';\n\
             import { state4 } from './hazard4';\n\
             export function svc() {}\n",
        )
        .unwrap();

        let test_path = base.join("trunc.test.ts");
        std::fs::write(
            &test_path,
            "import { svc } from './service';\ntest('svc', () => { svc(); });\n",
        )
        .unwrap();

        let source_files: Vec<std::path::PathBuf> = (1..=4)
            .map(|i| base.join(format!("hazard{i}.ts")))
            .chain(std::iter::once(service_path))
            .collect();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &source_files);

        let warnings: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability" && d.severity == isofence::rule::Severity::Warning)
            .collect();

        assert!(!warnings.is_empty(), "Expected transitive hazard warnings");

        let help = warnings[0].help.as_ref().unwrap();
        assert!(
            help.contains("and 1 more"),
            "Help with 4 modules should truncate to 3 + 'and 1 more', got: {help}"
        );
        assert!(
            help.contains("--json for full list"),
            "Truncated help should mention --json, got: {help}"
        );
    }

    #[test]
    fn diagnostics_only_on_test_files() {
        let dir = tempdir().unwrap();

        // Source file with hazards
        let source_path = dir.path().join("service.ts");
        std::fs::write(&source_path, "let state = 0;\nconst cache = new Map();\n").unwrap();

        // Test file
        let test_path = dir.path().join("app.test.ts");
        std::fs::write(&test_path, "import './service';\n").unwrap();

        let config = Config::load(dir.path().to_path_buf());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        // All diagnostics must be on test files
        for d in &result.diagnostics {
            assert!(
                is_test_file_path(&d.file_path),
                "Expected diagnostics only on test files, found on '{}': [{}] {}",
                d.file_path.display(), d.rule_name, d.message,
            );
        }

        // files_failed must not exceed files_checked
        assert!(
            result.files_failed <= result.files_checked,
            "files_failed ({}) should not exceed files_checked ({})",
            result.files_failed, result.files_checked,
        );
    }

    #[test]
    fn safe_only_import_skips_diagnostic() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // service.ts has mutable state + safe export
        let source_path = base.join("service.ts");
        std::fs::write(
            &source_path,
            r#"let counter = 0;
export function increment() { counter++; }
export const PI = 3.14;
"#,
        )
        .unwrap();

        // Test only imports the safe export
        let test_path = base.join("safe.test.ts");
        std::fs::write(
            &test_path,
            "import { PI } from './service';\ntest('pi', () => { expect(PI).toBe(3.14); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability.is_empty(),
            "Importing only safe export PI should NOT produce hazard-reachability diagnostic, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mutating_import_produces_diagnostic_with_details() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // service.ts has mutable state
        let source_path = base.join("service.ts");
        std::fs::write(
            &source_path,
            r#"let counter = 0;
export function increment() { counter++; }
export function getCount() { return counter; }
export const PI = 3.14;
"#,
        )
        .unwrap();

        // Test imports mutating and reading exports
        let test_path = base.join("mutation.test.ts");
        std::fs::write(
            &test_path,
            "import { increment, getCount } from './service';\ntest('inc', () => { increment(); expect(getCount()).toBe(1); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Importing mutating export should produce hazard-reachability diagnostic"
        );

        // Should have hazardous_imports details
        let diag = &reachability[0];
        assert!(
            !diag.hazardous_imports.is_empty(),
            "Diagnostic should include hazardous_imports details"
        );

        // Check that increment is classified as mutating
        let has_mutating = diag.hazardous_imports.iter().any(|hi| {
            hi.symbol_name == "increment"
                && hi.impact == MutationImpact::Mutating
        });
        assert!(
            has_mutating,
            "Should classify 'increment' as Mutating, got: {:?}",
            diag.hazardous_imports
                .iter()
                .map(|hi| format!("{}: {:?}", hi.symbol_name, hi.impact))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn namespace_import_reports_all_hazards() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        let source_path = base.join("service.ts");
        std::fs::write(
            &source_path,
            r#"let counter = 0;
export function increment() { counter++; }
export const PI = 3.14;
"#,
        )
        .unwrap();

        // Test uses namespace import — accesses all exports
        let test_path = base.join("ns.test.ts");
        std::fs::write(
            &test_path,
            "import * as svc from './service';\ntest('ns', () => { svc.increment(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Namespace import of module with hazards should produce diagnostic"
        );
    }

    #[test]
    fn non_exported_const_not_hazardous() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Module with non-exported mutable const + safe exported function
        // The const is mutable but NOT exported and NOT referenced by any export
        let source_path = base.join("internal.ts");
        std::fs::write(
            &source_path,
            r#"const internalCache = new Map();
export function greet(name: string) { return `Hello ${name}`; }
"#,
        )
        .unwrap();

        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import { greet } from './internal';\ntest('greet', () => { expect(greet('World')).toBe('Hello World'); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // Non-exported const not referenced by any export should not create a hazard
        assert!(
            reachability.is_empty(),
            "Non-exported mutable const (not referenced by exports) should NOT produce hazard-reachability diagnostic, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exported_const_still_hazardous() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Module with exported mutable const — should still be a hazard
        let source_path = base.join("state.ts");
        std::fs::write(
            &source_path,
            "export const handlers: Function[] = [];\n",
        )
        .unwrap();

        let test_path = base.join("handler.test.ts");
        std::fs::write(
            &test_path,
            "import { handlers } from './state';\ntest('handlers', () => { expect(handlers).toBeDefined(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Exported mutable const (empty array) should still produce hazard-reachability diagnostic"
        );
    }

    #[test]
    fn non_exported_const_referenced_by_mutating_export() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Module with non-exported mutable const referenced by exported mutating function
        let source_path = base.join("cache.ts");
        std::fs::write(
            &source_path,
            r#"const cache = new Map();
export function set(k: string, v: string) { cache.set(k, v); }
export function get(k: string) { return cache.get(k); }
"#,
        )
        .unwrap();

        let test_path = base.join("cache.test.ts");
        std::fs::write(
            &test_path,
            "import { set, get } from './cache';\ntest('cache', () => { set('a', 'b'); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // The non-exported const is referenced by the exported mutating functions,
        // so the module is still hazardous
        assert!(
            !reachability.is_empty(),
            "Non-exported const referenced by mutating export should still produce hazard-reachability"
        );
    }

    #[test]
    fn primitive_only_object_not_hazardous() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Module with exported primitive-only object — should be safe
        let source_path = base.join("constants.ts");
        std::fs::write(
            &source_path,
            r#"export const states = {
    Alabama: { code: 'AL', timezone: 'America/Chicago' },
    Alaska: { code: 'AK', timezone: 'America/Anchorage' }
};
"#,
        )
        .unwrap();

        let test_path = base.join("lookup.test.ts");
        std::fs::write(
            &test_path,
            "import { states } from './constants';\ntest('lookup', () => { expect(states.Alabama.code).toBe('AL'); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability.is_empty(),
            "Primitive-only object should NOT produce hazard-reachability diagnostic, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn enum_spread_array_not_hazardous() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // Module with enum spread array — should be safe
        let source_path = base.join("categories.ts");
        std::fs::write(
            &source_path,
            r#"enum SubCategory { A = 'A', B = 'B' }
enum MainCategory { X = 'X', Y = 'Y' }
export const allCategories = [...Object.values(SubCategory), ...Object.values(MainCategory)];
"#,
        )
        .unwrap();

        let test_path = base.join("cat.test.ts");
        std::fs::write(
            &test_path,
            "import { allCategories } from './categories';\ntest('cat', () => { expect(allCategories.length).toBeGreaterThan(0); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability.is_empty(),
            "Enum spread array should NOT produce hazard-reachability diagnostic, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn module_under_test_no_fix() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // invoice-filename-util.ts — has mutable state
        let source_path = base.join("invoice-filename-util.ts");
        std::fs::write(
            &source_path,
            "export let counter = 0;\nexport function formatName() { counter++; return 'inv'; }\n",
        )
        .unwrap();

        // invoice-filename-util.test.ts — tests the module above
        let test_path = base.join("invoice-filename-util.test.ts");
        std::fs::write(
            &test_path,
            "import { formatName } from './invoice-filename-util';\ntest('format', () => { expect(formatName()).toBe('inv'); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // Should have a diagnostic but NO fix (module-under-test)
        assert!(
            !reachability.is_empty(),
            "Should still produce a diagnostic for the hazardous module-under-test"
        );
        assert!(
            reachability.iter().all(|d| d.fix.is_none()),
            "Module-under-test diagnostics should NOT have a fix, got: {:?}",
            reachability.iter().map(|d| (&d.message, &d.fix)).collect::<Vec<_>>()
        );
        assert!(
            reachability.iter().all(|d| d.severity == isofence::rule::Severity::Warning),
            "Module-under-test diagnostics should be Warning severity"
        );
        let help = reachability[0].help.as_ref().unwrap();
        assert!(
            help.contains("module under test"),
            "Help should mention 'module under test', got: {help}"
        );
    }

    #[test]
    fn module_under_test_in_tests_dir_no_fix() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // foo.ts at project root
        let source_path = base.join("foo.ts");
        std::fs::write(&source_path, "export let state = 0;\n").unwrap();

        // __tests__/foo.test.ts
        let tests_dir = base.join("__tests__");
        std::fs::create_dir(&tests_dir).unwrap();
        let test_path = tests_dir.join("foo.test.ts");
        std::fs::write(
            &test_path,
            "import { state } from '../foo';\ntest('state', () => { expect(state).toBe(0); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[source_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Should have diagnostic for hazardous module"
        );
        assert!(
            reachability.iter().all(|d| d.fix.is_none()),
            "__tests__/foo.test.ts → ../foo.ts should be detected as module-under-test (no fix)"
        );
        assert!(
            reachability.iter().all(|d| d.severity == isofence::rule::Severity::Warning),
            "Module-under-test from __tests__ dir should be Warning"
        );
    }

    #[test]
    fn mock_consensus_transitive_no_fix() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // db.ts — hazardous
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let connection = null;\n").unwrap();

        // service.ts — imports db
        let service_path = base.join("service.ts");
        std::fs::write(
            &service_path,
            "import { connection } from './db';\nexport function getConn() { return connection; }\n",
        )
        .unwrap();

        // test-a.test.ts — mocks db directly
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./db');\nimport { connection } from './db';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — imports service (transitive dep on db), does NOT mock db
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { getConn } from './service';\ntest('b', () => { getConn(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[db_path, service_path],
        );

        // mock-consensus diagnostics on test-b for db.ts
        let consensus: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        // db.ts is transitively reachable from test-b via service → should have no fix
        for diag in &consensus {
            if diag.message.contains("db.ts") {
                assert!(
                    diag.fix.is_none(),
                    "mock-consensus for transitive module db.ts should NOT have a fix, got: {:?}",
                    diag.fix
                );
            }
        }
    }

    #[test]
    fn mock_consensus_direct_import_has_fix() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // config.ts — hazardous
        let config_path = base.join("config.ts");
        std::fs::write(&config_path, "export let dbUrl = 'localhost';\n").unwrap();

        // test-a.test.ts — mocks config
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./config');\nimport { dbUrl } from './config';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — imports config directly but doesn't mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { dbUrl } from './config';\ntest('b', () => { expect(dbUrl).toBeDefined(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[config_path],
        );

        // mock-consensus on test-b for config.ts (direct import) → should have fix
        let consensus: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| {
                d.rule_name == "mock-consensus"
                    && d.file_path == test_b
                    && d.message.contains("config.ts")
            })
            .collect();

        assert!(
            !consensus.is_empty(),
            "test-b should have mock-consensus diagnostic for config.ts"
        );
        assert!(
            consensus[0].fix.is_some(),
            "mock-consensus for directly imported config.ts should have a fix"
        );
    }

    #[test]
    fn mock_consensus_skips_module_under_test() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // foo.ts — has mutable state
        let foo_path = base.join("foo.ts");
        std::fs::write(&foo_path, "export let state = 0;\n").unwrap();

        // test-a.test.ts — mocks foo
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./foo');\nimport { state } from './foo';\ntest('a', () => {});\n",
        )
        .unwrap();

        // foo.test.ts — the module-under-test for foo.ts, imports but doesn't mock
        let test_foo = base.join("foo.test.ts");
        std::fs::write(
            &test_foo,
            "import { state } from './foo';\ntest('foo state', () => { expect(state).toBe(0); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_foo.clone()],
            &[foo_path],
        );

        // mock-consensus should NOT produce diagnostic on foo.test.ts for foo.ts
        let consensus_on_foo_test: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_foo)
            .collect();

        assert!(
            consensus_on_foo_test.is_empty(),
            "mock-consensus should skip module-under-test (foo.test.ts → foo.ts), got: {:?}",
            consensus_on_foo_test.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mock_consensus_skips_safe_only_imports() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // service.ts — has mutable state + safe export
        let source_path = base.join("service.ts");
        std::fs::write(
            &source_path,
            "let state = 0;\nexport function increment() { state++; }\nexport function safe() { return 42; }\n",
        )
        .unwrap();

        // test-a.test.ts — mocks service
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./service');\nimport { increment } from './service';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — only imports safe export, no mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { safe } from './service';\ntest('b', () => { expect(safe()).toBe(42); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[source_path],
        );

        // mock-consensus should NOT produce diagnostic on test-b for service.ts
        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            consensus_on_b.is_empty(),
            "mock-consensus should skip when test only imports safe exports, got: {:?}",
            consensus_on_b.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mock_consensus_keeps_unsafe_import_diagnostic() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // service.ts — has mutable state + safe export
        let source_path = base.join("service.ts");
        std::fs::write(
            &source_path,
            "let state = 0;\nexport function increment() { state++; }\nexport function safe() { return 42; }\n",
        )
        .unwrap();

        // test-a.test.ts — mocks service
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./service');\nimport { increment } from './service';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — imports the mutating export, no mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { increment } from './service';\ntest('b', () => { increment(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[source_path],
        );

        // mock-consensus SHOULD produce diagnostic on test-b (unsafe import)
        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            !consensus_on_b.is_empty(),
            "mock-consensus should still produce diagnostic when test imports unsafe exports"
        );
        assert!(
            consensus_on_b[0].fix.is_some(),
            "mock-consensus diagnostic for directly imported unsafe module should have a fix"
        );
    }

    #[test]
    fn side_effect_import_no_fix() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // setup.ts — hazardous module
        let setup_path = base.join("setup.ts");
        std::fs::write(&setup_path, "let initialized = false;\ninitialized = true;\n").unwrap();

        // test.test.ts — side-effect import only
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "import './setup';\ntest('app', () => {});\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[setup_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        // Should have diagnostic but NO fix and Warning severity
        assert!(
            !reachability.is_empty(),
            "Side-effect import of hazardous module should produce diagnostic"
        );
        assert!(
            reachability.iter().all(|d| d.fix.is_none()),
            "Side-effect import should NOT have a fix, got: {:?}",
            reachability.iter().map(|d| (&d.message, &d.fix)).collect::<Vec<_>>()
        );
        assert!(
            reachability.iter().all(|d| d.severity == isofence::rule::Severity::Warning),
            "Side-effect import diagnostic should be Warning severity"
        );
        let help = reachability[0].help.as_ref().unwrap();
        assert!(
            help.contains("Side-effect import"),
            "Help should mention 'Side-effect import', got: {help}"
        );
    }

    #[test]
    fn reexport_from_safe_module_no_diagnostic() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // safe-util.ts — no hazards, only safe exports
        let util_path = base.join("safe-util.ts");
        std::fs::write(
            &util_path,
            "export function add(a: number, b: number) { return a + b; }\n",
        )
        .unwrap();

        // barrel.ts — re-exports from safe-util
        let barrel_path = base.join("barrel.ts");
        std::fs::write(&barrel_path, "export { add } from './safe-util';\n").unwrap();

        // test imports from barrel
        let test_path = base.join("math.test.ts");
        std::fs::write(
            &test_path,
            "import { add } from './barrel';\ntest('add', () => { expect(add(1,2)).toBe(3); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[util_path, barrel_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            reachability.is_empty(),
            "Re-export from safe module should NOT produce hazard-reachability diagnostic, got: {:?}",
            reachability.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reexport_from_hazardous_module_still_reported() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // hazardous.ts — has mutable state
        let hazard_path = base.join("hazardous.ts");
        std::fs::write(
            &hazard_path,
            "let counter = 0;\nexport function increment() { counter++; }\n",
        )
        .unwrap();

        // barrel.ts — re-exports from hazardous
        let barrel_path = base.join("barrel.ts");
        std::fs::write(&barrel_path, "export { increment } from './hazardous';\n").unwrap();

        // test imports from barrel
        let test_path = base.join("inc.test.ts");
        std::fs::write(
            &test_path,
            "import { increment } from './barrel';\ntest('inc', () => { increment(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[hazard_path, barrel_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Re-export from hazardous module should still produce diagnostic"
        );
    }

    #[test]
    fn mock_consensus_skips_non_hazardous_module() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // utils.ts — pure function, no hazards
        let utils_path = base.join("utils.ts");
        std::fs::write(
            &utils_path,
            "export function add(a: number, b: number) { return a + b; }\n",
        )
        .unwrap();

        // test-a.test.ts — mocks utils (unnecessarily)
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./utils');\nimport { add } from './utils';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — uses utils without mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { add } from './utils';\ntest('b', () => { expect(add(1,2)).toBe(3); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[utils_path],
        );

        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            consensus_on_b.is_empty(),
            "mock-consensus should skip non-hazardous module, got: {:?}",
            consensus_on_b.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn mock_consensus_keeps_hazardous_module_diagnostic() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // db.ts — hazardous (mutable state)
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let connection = null;\n").unwrap();

        // test-a.test.ts — mocks db
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./db');\nimport { connection } from './db';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — uses db without mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { connection } from './db';\ntest('b', () => { expect(connection).toBeNull(); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[db_path],
        );

        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            !consensus_on_b.is_empty(),
            "mock-consensus should still report hazardous module"
        );
    }

    #[test]
    fn mock_consensus_guard_does_not_drop_reexport_hazards() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // hazardous.ts — mutable state
        let hazard_path = base.join("hazardous.ts");
        std::fs::write(&hazard_path, "export let state = 0;\n").unwrap();

        // barrel.ts — re-exports from hazardous
        let barrel_path = base.join("barrel.ts");
        std::fs::write(&barrel_path, "export { state } from './hazardous';\n").unwrap();

        // test-a.test.ts — mocks barrel
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./barrel');\nimport { state } from './barrel';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — uses barrel without mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { state } from './barrel';\ntest('b', () => { expect(state).toBe(0); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[hazard_path, barrel_path],
        );

        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            !consensus_on_b.is_empty(),
            "mock-consensus should keep diagnostic for barrel re-exporting hazardous module"
        );
    }

    #[test]
    fn mock_consensus_guard_skips_safe_reexport_barrel() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // safe-util.ts — no hazards
        let safe_path = base.join("safe-util.ts");
        std::fs::write(
            &safe_path,
            "export function add(a: number, b: number) { return a + b; }\n",
        )
        .unwrap();

        // barrel.ts — re-exports from safe-util
        let barrel_path = base.join("barrel.ts");
        std::fs::write(&barrel_path, "export { add } from './safe-util';\n").unwrap();

        // test-a.test.ts — mocks barrel
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./barrel');\nimport { add } from './barrel';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — uses barrel without mock
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "import { add } from './barrel';\ntest('b', () => { expect(add(1,2)).toBe(3); });\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[safe_path, barrel_path],
        );

        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            consensus_on_b.is_empty(),
            "mock-consensus should skip barrel that only re-exports safe modules, got: {:?}",
            consensus_on_b.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hazard_reachability_reset_modules_help_text() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // config.ts — hazardous
        let config_path = base.join("config.ts");
        std::fs::write(&config_path, "export let dbUrl = 'localhost';\n").unwrap();

        // test.test.ts — has resetModules but doesn't mock config
        let test_path = base.join("app.test.ts");
        std::fs::write(
            &test_path,
            "beforeEach(() => { vi.resetModules(); });\nimport { dbUrl } from './config';\ntest('app', () => {});\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(&[test_path], &[config_path]);

        let reachability: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "hazard-reachability")
            .collect();

        assert!(
            !reachability.is_empty(),
            "Should still produce hazard-reachability diagnostic even with resetModules"
        );
        let help = reachability[0].help.as_ref().unwrap();
        assert!(
            help.contains("resetModules()"),
            "Help should mention resetModules(), got: {help}"
        );
        assert!(
            help.contains("static imports"),
            "Help should warn about static imports, got: {help}"
        );
    }

    #[test]
    fn mock_consensus_help_changes_with_cleanup_signal() {
        let dir = tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();

        // db.ts — hazardous
        let db_path = base.join("db.ts");
        std::fs::write(&db_path, "export let connection = null;\n").unwrap();

        // test-a.test.ts — mocks db
        let test_a = base.join("test-a.test.ts");
        std::fs::write(
            &test_a,
            "vi.mock('./db');\nimport { connection } from './db';\ntest('a', () => {});\n",
        )
        .unwrap();

        // test-b.test.ts — has cleanup but doesn't mock db
        let test_b = base.join("test-b.test.ts");
        std::fs::write(
            &test_b,
            "afterEach(() => { vi.restoreAllMocks(); });\nimport { connection } from './db';\ntest('b', () => {});\n",
        )
        .unwrap();

        let config = Config::load(base.clone());
        let mut registry = RuleRegistry::new();
        registry.register_all(all_builtin_rules());

        let engine = Engine::new(config, registry);
        let result = engine.run_silent(
            &[test_a, test_b.clone()],
            &[db_path],
        );

        let consensus_on_b: Vec<_> = result
            .diagnostics
            .iter()
            .filter(|d| d.rule_name == "mock-consensus" && d.file_path == test_b)
            .collect();

        assert!(
            !consensus_on_b.is_empty(),
            "mock-consensus should still produce diagnostic"
        );
        let help = consensus_on_b[0].help.as_ref().unwrap();
        assert!(
            help.contains("Mock cleanup is present"),
            "Help should mention cleanup is present, got: {help}"
        );
    }
}

// ---- Export Mutation Analysis Parser Tests ----

mod export_analysis {
    use super::*;

    fn analyze(source: &str) -> Vec<(String, MutationImpact)> {
        let allocator = Allocator::default();
        let result = parse_source(&allocator, source, &path("module.ts"));
        assert!(!result.panicked, "Parse failed");

        let exports = extract_exports(&result.program);
        let mutable_bindings = collect_module_mutable_bindings(&result.program);
        let analyses = analyze_export_mutation(&result.program, &exports, &mutable_bindings);

        analyses
            .iter()
            .map(|a| (a.entry.exported_name.clone(), a.impact.clone()))
            .collect()
    }

    #[test]
    fn mutating_export_function() {
        let results = analyze(
            r#"let counter = 0;
export function increment() { counter++; }"#,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "increment");
        assert_eq!(results[0].1, MutationImpact::Mutating);
    }

    #[test]
    fn reading_export_function() {
        let results = analyze(
            r#"let counter = 0;
export function getCount() { return counter; }"#,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "getCount");
        assert_eq!(results[0].1, MutationImpact::Reading);
    }

    #[test]
    fn safe_export_const() {
        let results = analyze("export const PI = 3.14;");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "PI");
        assert_eq!(results[0].1, MutationImpact::Safe);
    }

    #[test]
    fn safe_pure_function() {
        let results = analyze(
            r#"let counter = 0;
export function add(a: number, b: number) { return a + b; }"#,
        );
        let add = results.iter().find(|r| r.0 == "add").unwrap();
        assert_eq!(add.1, MutationImpact::Safe);
    }

    #[test]
    fn shadowed_variable() {
        let results = analyze(
            r#"let counter = 0;
export function f() { let counter = 0; counter++; }"#,
        );
        let f = results.iter().find(|r| r.0 == "f").unwrap();
        assert_eq!(
            f.1,
            MutationImpact::Safe,
            "Shadowed variable should not trigger Mutating"
        );
    }

    #[test]
    fn collection_mutation() {
        let results = analyze(
            r#"const cache = new Map();
export function add(k: string, v: string) { cache.set(k, v); }"#,
        );
        let add = results.iter().find(|r| r.0 == "add").unwrap();
        assert_eq!(add.1, MutationImpact::Mutating);
    }

    #[test]
    fn reexport_is_unknown() {
        let results = analyze("export { foo } from './other';");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "foo");
        assert_eq!(results[0].1, MutationImpact::Unknown);
    }

    #[test]
    fn exported_mutable_binding_is_mutating() {
        let results = analyze("export let counter = 0;");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "counter");
        assert_eq!(results[0].1, MutationImpact::Mutating);
    }

    #[test]
    fn mixed_exports_classified_correctly() {
        let results = analyze(
            r#"let counter = 0;
export function increment() { counter++; }
export function getCount() { return counter; }
export const PI = 3.14;
export function pure(x: number) { return x + 1; }"#,
        );

        let inc = results.iter().find(|r| r.0 == "increment").unwrap();
        assert_eq!(inc.1, MutationImpact::Mutating);

        let get = results.iter().find(|r| r.0 == "getCount").unwrap();
        assert_eq!(get.1, MutationImpact::Reading);

        let pi = results.iter().find(|r| r.0 == "PI").unwrap();
        assert_eq!(pi.1, MutationImpact::Safe);

        let pure = results.iter().find(|r| r.0 == "pure").unwrap();
        assert_eq!(pure.1, MutationImpact::Safe);
    }

    #[test]
    fn arrow_function_export() {
        let results = analyze(
            r#"let state = 0;
export const mutate = () => { state = 1; };
export const read = () => state;
export const pure = () => 42;"#,
        );

        let mutate = results.iter().find(|r| r.0 == "mutate").unwrap();
        assert_eq!(mutate.1, MutationImpact::Mutating);

        let read = results.iter().find(|r| r.0 == "read").unwrap();
        assert_eq!(read.1, MutationImpact::Reading);

        let pure = results.iter().find(|r| r.0 == "pure").unwrap();
        assert_eq!(pure.1, MutationImpact::Safe);
    }

    #[test]
    fn export_default_function() {
        let results = analyze(
            r#"let counter = 0;
export default function increment() { counter++; }"#,
        );
        let default = results.iter().find(|r| r.0 == "default").unwrap();
        assert_eq!(default.1, MutationImpact::Mutating);
    }

    #[test]
    fn no_mutable_bindings_all_safe() {
        let results = analyze(
            r#"export const PI = 3.14;
export function add(a: number, b: number) { return a + b; }"#,
        );
        assert!(
            results.iter().all(|r| r.1 == MutationImpact::Safe),
            "All exports should be Safe when no mutable bindings exist"
        );
    }

    #[test]
    fn array_push_is_mutating() {
        let results = analyze(
            r#"const items: string[] = [];
export function addItem(item: string) { items.push(item); }"#,
        );
        let add_item = results.iter().find(|r| r.0 == "addItem").unwrap();
        assert_eq!(add_item.1, MutationImpact::Mutating);
    }

    #[test]
    fn object_property_assignment_is_mutating() {
        let results = analyze(
            r#"const config = {};
export function setKey(k: string, v: string) { config[k] = v; }"#,
        );
        let set_key = results.iter().find(|r| r.0 == "setKey").unwrap();
        assert_eq!(set_key.1, MutationImpact::Mutating);
    }
}
