import { createRoot, createSignal } from 'solid-js';
import { describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../api/client';
import { useBranchCountable } from './useBranchCountable';

describe('useBranchCountable', () => {
  it('loads branch_currencies and extracts codes', async () => {
    const fakeApi = {
      view: vi.fn().mockResolvedValue({
        data: {
          rows: [
            { code: 'THB', group: 'NOTE', display: 'SHOW', sortOrder: 1, held: true },
            { code: 'USD', group: 'NOTE', display: 'SHOW', sortOrder: 2, held: true },
          ],
        },
      }),
    } as unknown as ApiClient;

    await createRoot(async (dispose) => {
      const [branch] = createSignal<string | null>('BKK');
      const countable = useBranchCountable(fakeApi, branch);

      await vi.waitFor(() => expect(countable()).toEqual(['THB', 'USD']));
      expect(fakeApi.view).toHaveBeenCalledWith('branch_currencies', { branch: 'BKK' });
      dispose();
    });
  });

  it('tolerates bare array response', async () => {
    const fakeApi = {
      view: vi.fn().mockResolvedValue({
        data: [
          { code: 'EUR', group: 'NOTE', display: 'SHOW', sortOrder: 1, held: false },
        ],
      }),
    } as unknown as ApiClient;

    await createRoot(async (dispose) => {
      const [branch] = createSignal<string | null>('BKK');
      const countable = useBranchCountable(fakeApi, branch);

      await vi.waitFor(() => expect(countable()).toEqual(['EUR']));
      dispose();
    });
  });

  it('sets countable to undefined on error', async () => {
    const fakeApi = {
      view: vi.fn().mockRejectedValue(new Error('Network error')),
    } as unknown as ApiClient;

    await createRoot(async (dispose) => {
      const [branch] = createSignal<string | null>('BKK');
      const countable = useBranchCountable(fakeApi, branch);

      await vi.waitFor(() => expect(fakeApi.view).toHaveBeenCalled());
      expect(countable()).toBeUndefined();
      dispose();
    });
  });

  it('returns undefined if branch is null', async () => {
    const fakeApi = {
      view: vi.fn(),
    } as unknown as ApiClient;

    await createRoot(async (dispose) => {
      const [branch] = createSignal<string | null>(null);
      const countable = useBranchCountable(fakeApi, branch);

      expect(countable()).toBeUndefined();
      expect(fakeApi.view).not.toHaveBeenCalled();
      dispose();
    });
  });
});
