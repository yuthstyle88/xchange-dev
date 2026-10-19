// A03 rate publication history (survey xchange#564 row A03; BE #3399). The local shape of one row of the paged
// `rate_publications` view, as #3399 §7 states it ([required] there). It stands in for the generated
// `web/src/shared/api/generated/rate_publications.ts` until #3399 lands; then this file re-exports those types.

export type RateSides = { buy: string | null; sell: string | null };

export type RatePublicationLine = {
  currency: string;
  group: string;
  /** As stored; `ALL` means no tier. */
  valueTier: string;
  unitSize: string;
  /** The same (currency, group, valueTier) in the branch's previous version; null when the line is new. */
  before: RateSides | null;
  /** This version's line; null when the line was dropped. */
  after: RateSides | null;
};

export type RatePublicationRow = {
  id: string;
  branchId: string;
  version: number;
  publishedAt: string;
  publishedBy: string;
  approvedRequestId: string | null;
  draftId: string | null;
  /** `PUBLISHED` for the branch's current publication, else `SUPERSEDED`. */
  state: string;
  lines: RatePublicationLine[];
};

export type LineChange = 'added' | 'dropped' | 'changed' | 'same';

export function lineChange(line: RatePublicationLine): LineChange {
  if (line.before === null) return 'added';
  if (line.after === null) return 'dropped';
  if (line.before.buy !== line.after.buy || line.before.sell !== line.after.sell) {
    return 'changed';
  }
  return 'same';
}
