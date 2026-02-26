# Roadmap

Current version: **0.1.0** (initial release)

## Status Overview

| Phase | Description | Status |
|-------|-------------|--------|
| Foundation | Rule trait, parser, registry | Done |
| Built-in Rules | 11 hazard detection rules | Done |
| Engine | 3-phase pipeline, graph, resolver | Done |
| Mock Consensus | Cross-test mock consistency | Done |
| Declarative Rules | JSON custom rules | Done |
| CLI + Config | clap, isofence.json, auto-detect | Done |
| Auto-Fix | `--fix`, `--dry-run`, mock insertion | Done |
| Reporting | Console + JSON output, exit codes | Done |

---

## 0.2.0 — Analysis Accuracy

Improve detection precision and reduce false positives/negatives.

### Depth-limited transitive analysis
Currently the graph traversal is unbounded. The `--depth` flag is accepted but not enforced during effective subgraph traversal. Implement depth-limited BFS so `--depth 1` only checks direct dependencies.

### `vi.doMock` / dynamic import handling
`vi.doMock` is not hoisted — it only affects subsequent `await import()` calls. Currently treated conservatively as a full mock. Properly model the non-hoisting behavior and scope it to dynamic imports only.

### `__mocks__` directory auto-detection
Jest/Vitest support automatic mocking via `__mocks__/` directories adjacent to modules. Detect these and treat them as implicit mock declarations.

### `vi.unmock()` / `jest.unmock()` handling
Track unmock calls that cancel previous mock declarations. Currently not parsed.

### Confidence levels in reporting
Distinguish between `Definite` (e.g., `let` at module scope) and `Potential` (e.g., TypeScript `enum` — conventionally immutable). Surface confidence in output so users can filter by certainty.

### HazardCategory accuracy
Currently all hazards from Phase 1/2 are tagged as `MutableState`. Properly classify as `MutableState` vs `SideEffect` based on the originating rule.

---

## 0.3.0 — Richer Fix & Output

### Factory mock generation
Current `--fix` inserts bare `vi.mock('./path')` (automock). Generate factory mocks with stub implementations based on the module's exports:
```typescript
vi.mock('./database', () => ({
  query: vi.fn(),
  connect: vi.fn(),
}));
```

### Import chain visualization
When reporting hazards, show the full import chain from test to hazardous module:
```
test.ts → service.ts → repository.ts → database.ts
                                        ↑ hazard: const pool = new Pool()
```

### Cross-test shared state detection
Detect when 2+ test files reach the same hazardous module without mocking it. These are the most dangerous cases — they cause order-dependent test failures.

### Interactive fix mode
`--fix --interactive`: prompt per-module to mock, skip, or add to allowlist.

### Post-fix formatter integration
After `--fix`, optionally run `prettier` or `eslint --fix` to format the inserted code.

---

## 0.4.0 — Extensibility

### WASM plugin rules
Allow custom rules written in Rust (or any language compilable to WASM) for complex detection logic that JSON declarative rules can't express.

### Declarative rule enhancements
- Message template interpolation (`{{source}}`, `{{callee}}`, `{{name}}`)
- Compound matchers (AND/OR)
- Negative matchers (NOT)
- Import source pattern matching

### Programmatic API
Expose the engine as a library for integration into other tools. Stabilize the `Rule` trait for external consumers.

---

## 0.5.0 — Ecosystem

### npm distribution
Publish to npm with platform-specific binary downloads (like `@biomejs/biome` or `@oxlint/oxlint`). `npx isofence` should just work.

### GitHub Action
Official action for CI integration:
```yaml
- uses: selenehyun/isofence-action@v1
  with:
    strict: true
```

### Editor integration
- VSCode extension with inline diagnostics
- LSP server for real-time feedback

### Monorepo support
Handle multiple `tsconfig.json` files and workspace boundaries in monorepo setups (turborepo, nx, lerna).

---

## Future Considerations

These are ideas under evaluation, not committed work.

### `oxc_semantic` integration
Use OXC's semantic analysis to determine which exports reference mutable state. This enables per-export hazard tracking instead of per-module, significantly reducing false positives for modules that export both pure functions and mutable state.

### Dynamic import graph edges
Track `await import('./module')` as `EdgeKind::DynamicImport` with lower confidence, since dynamic imports may be conditional.

### CommonJS support
Detect `require()` calls and `module.exports` patterns for projects that mix ESM and CJS.

### Watch mode
`isofence --watch` for real-time feedback during development.

### Baseline / suppress
Allow suppressing known issues with inline comments (`// isofence-ignore-next-line`) or a baseline file, similar to how type-coverage tools work.
