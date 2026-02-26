// Should detect: const with mutable initializers
const cache = new Map<string, string>();
const items: string[] = [];
const state = { count: 0 };
const client = new HttpClient();
const logger = createLogger();
const RE = /pattern/g;

// Should NOT detect: const with safe initializers
const MAX_RETRIES = 3;
const API_URL = "https://api.example.com";
const ENABLED = true;
const NOTHING = null;
const UNDEF = undefined;
const TEMPLATE = `hello ${1 + 2}`;
const FROZEN = Object.freeze({ a: 1 });
const TYPED = { a: 1 } as const;

export { cache, items, state, client, logger, RE };
export { MAX_RETRIES, API_URL, ENABLED, NOTHING, UNDEF, TEMPLATE, FROZEN, TYPED };
