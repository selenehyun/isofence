// Should detect: side-effect imports (no specifiers)
import './setup';
import './polyfill';
import type { Config } from './types'; // Safe — type-only

export function run() {}
