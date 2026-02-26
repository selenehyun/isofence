# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
cargo build --release          # optimized build → target/release/isofence
cargo test                     # all 86 tests (9 unit + 18 engine + 59 rules)
cargo test mutable_const_init  # run tests matching name
cargo test graph::             # run tests matching pattern
cargo test -- --nocapture      # show stdout/stderr during tests
```

Run the binary against a TypeScript project:
```bash
cargo run --release -- /path/to/ts-project
./target/release/isofence /path/to/ts-project --fix --dry-run
```

## Architecture

IsoFence is a single-crate Rust binary that statically analyzes TypeScript test files for state isolation hazards.

### 3-Phase Engine Pipeline (`src/engine/mod.rs`)

```
Phase 1: check_module_item(stmt, ctx)  → per top-level statement  (rayon parallel)
Phase 2: check_module(ctx)             → per file                  (rayon parallel)
Phase 3: check_graph(graph_ctx)        → project-wide              (sequential)
```

- Phases 1&2 run on ALL files (test + source) in parallel via rayon
- Phase 3 runs once after all files are analyzed — builds ModuleGraph with oxc-resolver, constructs mock_registry from all test files, then runs graph-level rules
- After Phase 3, `run_hazard_reachability()` computes effective subgraph per test file and emits diagnostics for unmocked hazardous modules
- Allowlist filtering happens post-analysis: `module_hazards` and `mock_registry` are filtered before graph analysis

### Rule Trait (`src/rule/mod.rs`)

```rust
pub trait Rule: Send + Sync {
    fn meta(&self) -> RuleMeta;
    fn check_module_item(&self, stmt: &Statement, ctx: &ModuleContext) -> Vec<Diagnostic>;  // Phase 1
    fn check_module(&self, ctx: &ModuleContext) -> Vec<Diagnostic>;                          // Phase 2
    fn check_graph(&self, ctx: &GraphContext) -> Vec<Diagnostic>;                            // Phase 3
}
```

All 3 hooks have default empty implementations. Most rules only implement `check_module_item`. Only `side-effect-import` uses `check_module`, and only `mock-consensus` uses `check_graph`.

### Key Data Flow

1. `Engine::analyze_file()` → parses with OXC, builds `ModuleContext`, runs Phase 1&2 rules, collects `Hazard`s from source file diagnostics
2. `Engine::build_graph()` → resolves imports with oxc-resolver, builds `ModuleGraph` with edges
3. `ModuleGraph::effective_subgraph()` → BFS with mock overlay (full mocks cut edges, partial mocks pass through, type-only imports skipped)
4. `Diagnostic.fix` field → consumed by `fix/mod.rs` to generate `vi.mock()`/`jest.mock()` insertions

## OXC 0.56 API Notes

These are non-obvious and will cause compile errors if forgotten:

- `Statement` needs `use oxc_span::GetSpan` to call `.span()`
- `BindingPattern::get_identifier_name()` returns `Option<Atom<'_>>` — use `.map(|a| a.to_string())`
- `ExportDefaultDeclarationKind` — use `.as_expression()` to get the inner expression
- `similar` crate requires `features = ["inline"]` for `iter_inline_changes`
- Parser allocator has lifetime constraints: `Allocator` must outlive `ParserReturn`

## Adding a New Rule

1. Create `src/rules/your_rule.rs` implementing `Rule` trait
2. Always early-return for test files: `if ctx.is_test_file { return vec![]; }`
3. Register in `src/rules/mod.rs` → `all_builtin_rules()` vec
4. Add tests in `tests/rules_test.rs` using the `check_rule(source, &YourRule)` helper — test both detection and safe patterns

Test helper returns diagnostic messages as `Vec<String>`:
```rust
#[test]
fn detects_pattern() {
    let msgs = check("let x = 1;");
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].contains("let x"));
}
```

## Module Responsibilities

- `src/config.rs` — Loads optional `isofence.json`, auto-detects framework from package.json, glob matching for allowlist
- `src/engine/graph.rs` — `ModuleGraph` with `effective_subgraph()` (BFS + mock overlay) and `shortest_path()`
- `src/engine/parser.rs` — OXC wrapper + all AST classification helpers (`is_safe_const_init`, `is_mutable_const_init`, `is_as_const`, etc.)
- `src/fix/` — Auto-fix orchestration: groups diagnostics by test file, computes insertion point (after last mock/import), deduplicates, generates relative paths
- `src/rule/declarative/` — Compiles JSON rule definitions into `Rule` trait implementations
