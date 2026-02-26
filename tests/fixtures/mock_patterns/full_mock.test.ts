import { describe, it, vi, expect } from 'vitest';

vi.mock('./database');
vi.mock('./logger', () => ({
  log: vi.fn(),
}));

import { query } from './database';
import { log } from './logger';

describe('full mock test', () => {
  it('should work', () => {
    expect(query).toBeDefined();
  });
});
