// The currencies a rate draft can add: the reference source's codes not yet in the draft (xchange#564 D-a gap).
// Design: docs/design/rates-draftable-currencies.md §2. Skeleton: DC-1 writes the body.
import type { RatesReferenceData } from '../../../shared/api/generated/rates_reference';

export function draftableCurrencies(reference: RatesReferenceData | null, inDraft: readonly string[]): string[] {
  void reference;
  void inDraft;
  return [];
}
