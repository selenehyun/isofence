import { describe, it, expect } from 'vitest';

// No mocks at all — should flag hazardous imports
import { query } from './database';
import { cache } from '../mutable_state/const_mutable';

describe('no mock test', () => {
  it('uses real database', () => {
    expect(query).toBeDefined();
  });
});
