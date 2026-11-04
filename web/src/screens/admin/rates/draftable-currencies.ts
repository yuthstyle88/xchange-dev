// The currencies a rate draft can add: the branch's draftable rows not yet in the draft (docs/design/board-currencies.md § BC-6).
import type { RatesDraftableData } from '../../../shared/api/generated/rates_draftable';

export function draftableCurrencies(data: RatesDraftableData | null, inDraft: readonly string[]): string[] {
  if (!data?.rows) return [];
  const skip = new Set(['', 'THB', ...inDraft.map((c) => c.trim().toUpperCase())]);
  const out = new Set<string>();
  for (const row of data.rows) {
    const code = (row.code ?? '').trim().toUpperCase();
    if (!skip.has(code)) out.add(code);
  }
  return [...out].sort();
}
