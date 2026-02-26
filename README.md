# IsoFence

**Reference Graph based test isolation verification for TypeScript.**

IsoFence builds a module dependency graph from your TypeScript test files, applies mock overlays, and detects state-sharing hazards in the **effective subgraph** — the modules that actually execute at test time. It catches the subtle bugs that occur when tests share mutable state through unmocked imports.

```
isofence v0.1.0

✗ src/services/__tests__/user.service.test.ts

  ├── ⚠ hazard-reachability: Unmocked hazardous module `user.repository.ts`:
  │     `const cache` — collection constructor (Map/Set/etc.) is mutable
  │   → This module has 3 hazard(s). Mock it to isolate your tests.
  │
  └── ⚠ mock-consensus: `database.ts` is mocked in [auth.test.ts, payment.test.ts]
        but not in this test
      → This module is mocked in other tests, suggesting it needs isolation.

✓ src/controllers/__tests__/auth.controller.test.ts

Summary: 2 files checked, 1 passed, 1 failed
```

## Why IsoFence?

In non-isolated test runners (the default for Vitest and Jest), **all tests share the same module cache**. This means:

```typescript
// cache.ts — module scope
const cache = new Map();  // shared across ALL tests

// test-a.test.ts
import { cache } from './cache';  // gets the shared instance
cache.set('key', 'from-test-a');

// test-b.test.ts
import { cache } from './cache';  // same instance!
cache.get('key');  // → 'from-test-a' — test pollution!
```

If `test-a` runs first, it contaminates `test-b`. These bugs are order-dependent, flaky, and extremely hard to track down.

**IsoFence detects these hazards statically** — before your tests even run:

1. Builds the **reference graph** of all module dependencies
2. Applies your **mock declarations** as edge cuts in the graph
3. Analyzes the **effective subgraph** (what actually loads at test time)
4. Reports **hazardous modules** that remain unmocked

## Quick Start

```bash
# Install
cargo install isofence

# Run (zero-config — auto-detects everything)
isofence

# Check specific directories
isofence src/services/ src/controllers/

# Auto-fix: insert missing mock declarations
isofence --fix --dry-run   # preview first
isofence --fix              # apply
```

No configuration file needed. IsoFence auto-detects:
- Test files (`*.test.ts`, `*.spec.ts`, `__tests__/**`)
- Test framework (Vitest/Jest) from `package.json`
- Path aliases from `tsconfig.json`
- Ignored paths from `.gitignore`

### Build from Source (Dev)

```bash
git clone https://github.com/selenehyun/isofence.git
cd isofence
cargo build --release

# Register as a shell command (zsh)
echo 'alias isofence-dev="'"$(pwd)"'/target/release/isofence"' >> ~/.zshrc
```

Open a new terminal, then run `isofence-dev` from any TypeScript project. After code changes, `cargo build --release` updates the binary in-place — no reinstall needed.

## How It Works

### 1. Module Hazard Detection

IsoFence analyzes each module's **top-level scope** for state-sharing risks:

| Hazard | Example | Why it's dangerous |
|--------|---------|-------------------|
| Mutable binding | `let counter = 0` | Shared state between tests |
| Mutable const | `const cache = new Map()` | `const` prevents reassignment, not mutation |
| Top-level call | `initializeApp()` | Side effect executes on every import |
| Global mutation | `process.env.NODE_ENV = 'test'` | Affects all modules |
| Event subscription | `emitter.on('data', handler)` | Persists across tests |
| IIFE | `(() => { setup() })()` | Immediate execution on import |
| Static class field | `static instances = new Map()` | Shared mutable class state |
| Prototype mutation | `Array.prototype.flat = ...` | Global prototype pollution |
| Top-level await | `const db = await connect()` | Async side effect on import |
| Side-effect import | `import './polyfill'` | Executes module for side effects |

Modules with only safe patterns are **not flagged**:
- `const MAX = 3` (primitive)
- `const CONFIG = { ... } as const` (readonly)
- `const FROZEN = Object.freeze({ ... })` (immutable)
- `export function add(a, b) { ... }` (declaration only)
- `export type User = { ... }` (type-only, no runtime)

### 2. Reference Graph & Mock Overlay

```
test.ts ──import──→ service.ts ──import──→ database.ts ──import──→ cache.ts
           │
           └─ vi.mock('./service')  ← edge cut (Full Mock)

Effective subgraph = { test.ts }
                     service.ts is mocked → its subtree is pruned
```

- **Full mock** (`vi.mock('./path')`) → edge cut, subtree pruned
- **Partial mock** (`vi.mock('./path', async (importOriginal) => ...)`) → module still loads
- **Type-only import** (`import type { ... }`) → automatically excluded

### 3. Mock Consensus

If module X is mocked in **any** test file, IsoFence flags other test files that use X without mocking it. The reasoning: if one developer decided X needs isolation, it probably does everywhere.

```
test-a.test.ts:  vi.mock('./database')     ← mocked here
test-b.test.ts:  import { query } from '../database'  ← no mock!
→ mock-consensus warning on test-b
```

## Configuration

Configuration is **optional**. Create `isofence.json` only when you need to customize defaults:

```bash
isofence --init  # generate template
```

```jsonc
// isofence.json
{
  // Safe module patterns (always skipped)
  "allowlist": [
    "src/types/**",
    "src/constants/**"
  ],

  // Rule severity overrides
  "rules": {
    "mutable-module-var": "error",
    "top-level-call": "warning",
    "side-effect-import": "warning",
    "mock-consensus": "warning",
    "iife": "off"
  },

  // Custom declarative rules (JSON)
  "customRules": ["./isofence-rules/firebase.json"]
}
```

### Custom Declarative Rules

Define project-specific rules in JSON:

```jsonc
// isofence-rules/firebase.json
{
  "rules": [
    {
      "name": "custom/no-prisma-client",
      "description": "PrismaClient at module scope creates shared DB connection",
      "severity": "error",
      "match": {
        "type": "var_decl",
        "kind": "const",
        "init": { "type": "new", "callee": { "name": "PrismaClient" } }
      },
      "message": "PrismaClient at module scope shares DB connection between tests."
    }
  ]
}
```

## CLI Reference

```
isofence [paths...]              # files or directories (default: .)

Options:
      --fix                Auto-insert missing mock declarations
      --dry-run            Preview fixes without applying (use with --fix)
      --format <FMT>       Output format: console, json (default: console)
  -d, --depth <N>          Transitive dependency check depth (default: 1)
      --strict             Treat all findings as errors (CI mode)
      --no-consensus       Disable mock consensus check
      --tsconfig <PATH>    Path to tsconfig.json (usually auto-detected)
      --init               Generate isofence.json template
  -q, --quiet              Only show files with issues
```

### CI Integration

```yaml
# GitHub Actions
- name: Test isolation check
  run: |
    cargo install isofence
    isofence --strict --format json > isofence-report.json
```

Exit codes: `0` pass, `1` violations found, `2` internal error.

### Auto-Fix

IsoFence can automatically insert missing `vi.mock()` / `jest.mock()` declarations:

```bash
# Preview what would change
isofence --fix --dry-run

# Apply fixes
isofence --fix
```

```diff
 import { query } from '../database';
 import { cache } from '../services/cache';
+vi.mock('../database');
+vi.mock('../services/cache');

 describe('user service', () => {
```

Fixes are **idempotent** (won't duplicate existing mocks) and **format-preserving** (only inserts new lines).

## Built-in Rules

| Rule | Default | Phase | Description |
|------|---------|-------|-------------|
| `mutable-module-var` | error | item | `let`/`var` declarations at module scope |
| `mutable-const-init` | error | item | `const` with mutable initializer (object, array, Map, Set, etc.) |
| `top-level-call` | error | item | Function calls at module scope |
| `global-mutation` | error | item | Assignments to `globalThis`/`process`/`window` |
| `event-subscription` | error | item | `.on()`/`.addEventListener()`/`.subscribe()` at module scope |
| `top-level-await` | error | item | `await` at module scope |
| `iife` | error | item | Immediately invoked function expressions |
| `prototype-mutation` | error | item | `*.prototype.*` assignments |
| `side-effect-import` | warning | module | `import './setup'` (no specifiers) |
| `static-class-field` | error | item | Static class fields with mutable initializers |
| `mock-consensus` | warning | graph | Module mocked elsewhere but not in this test |

## Architecture

```
                      ┌─────────────────────────────────┐
                      │         isofence CLI             │
                      └──────────────┬──────────────────┘
                                     │
                      ┌──────────────▼──────────────────┐
                      │           Engine                 │
                      │                                  │
                      │  Phase 1: check_module_item()    │──→ per statement (parallel)
                      │  Phase 2: check_module()         │──→ per file     (parallel)
                      │  Phase 3: check_graph()          │──→ project-wide (sequential)
                      │                                  │
                      └──┬────────────┬────────────┬────┘
                         │            │            │
              ┌──────────▼──┐  ┌──────▼─────┐  ┌──▼──────────┐
              │  OXC Parser  │  │ oxc-resolver│  │ Rule Registry│
              │  (AST)       │  │ (paths)     │  │ (11 built-in)│
              └─────────────┘  └────────────┘  └─────────────┘
```

Built on [OXC](https://oxc.rs) for parsing and module resolution. File analysis is parallelized with [rayon](https://docs.rs/rayon).

## Design Principles

| Principle | Description |
|-----------|-------------|
| **Zero-config** | Works out of the box. `isofence` — that's it. |
| **Non-invasive** | Read-only by default. No dependencies added to your project. |
| **Single binary** | One executable. No node_modules pollution. |
| **Convention-based** | Auto-detects test files, framework, paths, ignores. |
| **Easy in, easy out** | Install: `cargo install`. Remove: delete the binary. No traces. |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

[MIT](LICENSE) - Tim Kang
