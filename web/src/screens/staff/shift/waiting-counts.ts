// The branch's WAITING cash counts, read the way ShiftCard read them (docs/design/desk-shift-open.md §2): once on
// mount, again on each fresh `cash_count` notice (#5564), with `loaded` false until the first answer (#5591).
// Call it inside a component: it registers on mount and stops listening on cleanup.
import { createSignal, onCleanup, onMount, type Accessor } from 'solid-js';
import type { ApiClient } from '../../../shared/api/client';
import type { CashCountRow, CashCountsFilter } from '../../../shared/api/generated/cash_counts';
import { wordedError } from '../../../shared/i18n/worded-error';
import { onFreshNotice } from '../../../shared/ui/notice-bell/notice-feed';

export interface WaitingCounts {
  /** The branch's WAITING counts from the last good answer. */
  rows: Accessor<CashCountRow[]>;
  /** True once the first read has answered, well or not; never false again. */
  loaded: Accessor<boolean>;
  /** The worded failure of the last read; null after a good one. */
  error: Accessor<string | null>;
  /** Reads again (no-op while `enabled()` is false). */
  reload(): Promise<void>;
}

export function createWaitingCounts(opts: {
  api: ApiClient;
  branch: Accessor<string>;
  enabled: Accessor<boolean>;
}): WaitingCounts {
  const [rows, setRows] = createSignal<CashCountRow[]>([]);
  const [loaded, setLoaded] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const reload = async (): Promise<void> => {
    if (!opts.enabled()) return;
    try {
      const res = await opts.api.view<CashCountRow[] | { rows: CashCountRow[] }, Partial<CashCountsFilter>>(
        'cash_counts',
        { branch: opts.branch(), filter: { status: 'WAITING' } },
      );
      const data = res.data;
      if (Array.isArray(data)) {
        setRows(data);
      } else if (data && Array.isArray((data as { rows: CashCountRow[] }).rows)) {
        setRows((data as { rows: CashCountRow[] }).rows);
      }
      setError(null);
    } catch (err: unknown) {
      setError(wordedError(err));
    } finally {
      setLoaded(true);
    }
  };

  onMount(() => {
    void reload();
    onCleanup(onFreshNotice('cash_count', () => void reload()));
  });

  return { rows, loaded, error, reload };
}
