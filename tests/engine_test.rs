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
