import { describe, it, vi, expect } from 'vitest';

vi.mock('./api', async (importOriginal) => {
  const actual = await importOriginal();
  return {
    ...actual,
    fetchData: vi.fn(),
  };
});

import { fetchData, apiClient } from './api';

describe('partial mock test', () => {
  it('should override fetchData but keep apiClient', () => {
    expect(fetchData).toBeDefined();
  });
});
