# Contributing to IsoFence

## Development Setup

### Prerequisites

- Rust 1.75+ (`rustup` recommended)
- Node.js 18+ (for test fixtures only)

### Build

```bash
git clone https://github.com/selenehyun/isofence.git
cd isofence
cargo build
```

### Test

```bash
# Run all tests
cargo test

# Run specific test module
cargo test mutable_const_init
cargo test graph::

# Run with output
cargo test -- --nocapture
```

### Run locally

```bash
# Debug build
cargo run -- src/

# Release build
cargo build --release
./target/release/isofence src/
```

## Project Structure

```
src/
├── main.rs                 # CLI entry point
├── lib.rs                  # Public API
├── config.rs               # Configuration loading
├── engine/                 # Core analysis engine
│   ├── mod.rs              #   3-phase pipeline orchestration
│   ├── parser.rs           #   OXC parser wrapper + AST helpers
│   ├── context.rs          #   ModuleContext, GraphContext
│   ├── graph.rs            #   ModuleGraph + effective subgraph
│   └── fixer.rs            #   Span-based text insertion
├── rule/                   # Rule system
│   ├── mod.rs              #   Rule trait, Diagnostic, Severity
│   ├── registry.rs         #   Rule registration + config
│   └── declarative/        #   JSON rule compiler
├── rules/                  # Built-in rules (1 file = 1 rule)
├── fix/                    # Auto-fix logic
└── reporter/               # Output formatters
```

## Adding a New Rule

1. Create `src/rules/your_rule.rs`
2. Implement the `Rule` trait:

```rust
use crate::engine::context::ModuleContext;
use crate::rule::{Diagnostic, HazardCategory, Rule, RuleMeta, Severity};
use oxc_ast::ast::Statement;

pub struct YourRule;

impl Rule for YourRule {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            name: "your-rule",
            description: "What this rule detects",
            category: HazardCategory::MutableState, // or SideEffect
            default_severity: Severity::Error,
        }
    }

    fn check_module_item(
        &self,
        stmt: &Statement<'_>,
        ctx: &ModuleContext,
    ) -> Vec<Diagnostic> {
        if ctx.is_test_file {
            return vec![];
        }
        // Your detection logic here
        vec![]
    }
}
```

3. Register in `src/rules/mod.rs`:

```rust
pub mod your_rule;

pub fn all_builtin_rules() -> Vec<Box<dyn Rule>> {
    vec![
        // ... existing rules
        Box::new(your_rule::YourRule),
    ]
}
```

4. Add tests in `tests/rules_test.rs`
5. Add fixture files in `tests/fixtures/` if needed

## Rule Hooks

| Hook | When | Use case |
|------|------|----------|
| `check_module_item(stmt, ctx)` | Each top-level statement | Most rules (80%) |
| `check_module(ctx)` | After all statements | Needs full file context |
| `check_graph(graph_ctx)` | After all files processed | Cross-file analysis |

## Code Conventions

- One rule per file in `src/rules/`
- Skip test files early: `if ctx.is_test_file { return vec![]; }`
- Use `Severity::Error` for definite hazards, `Severity::Warning` for potential issues
- Include `help` text in diagnostics suggesting how to fix the issue
- Test both positive (detection) and negative (no false positive) cases

## Testing

Tests are organized in two files:

- `tests/rules_test.rs` — Unit tests for individual rules
- `tests/engine_test.rs` — Integration tests for parser, context, graph

The `check_rule()` helper parses source and runs a single rule:

```rust
fn check(source: &str) -> Vec<String> {
    check_rule(source, &YourRule)
}

#[test]
fn detects_the_pattern() {
    let msgs = check("let x = 1;");
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].contains("let x"));
}

#[test]
fn ignores_safe_pattern() {
    assert!(check("const x = 1;").is_empty());
}
```

## Pull Requests

- Keep PRs focused on a single change
- Include tests for new rules or behavior changes
- Run `cargo test` before submitting
- Run `cargo clippy` to check for lint warnings
