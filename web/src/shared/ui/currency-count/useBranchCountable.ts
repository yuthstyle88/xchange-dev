import { createEffect, createSignal, type Accessor } from 'solid-js';
import type { ApiClient } from '../../api/client';
import type { BranchCurrenciesData, BranchCurrencyRow } from '../../api/generated/branch_currencies';

export function useBranchCountable(
  api: ApiClient,
  branch: Accessor<string | null>,
): Accessor<string[] | undefined> {
  const [countable, setCountable] = createSignal<string[] | undefined>(undefined);

  createEffect(() => {
    const b = branch();
    if (!b) {
      setCountable(undefined);
      return;
    }
    void (async () => {
      try {
        const res = await api.view<BranchCurrenciesData | BranchCurrencyRow[]>('branch_currencies', {
          branch: b,
        });
        if (branch() !== b) return;
        const data = res.data;
        const rows = Array.isArray(data) ? data : data?.rows;
        if (Array.isArray(rows)) {
          setCountable(rows.map((r) => r.code));
        }
      } catch {
        if (branch() !== b) return;
        setCountable(undefined);
      }
    })();
  });

  return countable;
}
