use isofence::engine::context::{
    is_test_file_path, EdgeKind, MockDeclaration, MockKind, TestFramework,
};
use isofence::engine::graph::ModuleGraph;
use isofence::engine::parser::{
    extract_imports, extract_mocks, parse_source,
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
    use isofence::engine::context::is_test_file_path;
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
}
