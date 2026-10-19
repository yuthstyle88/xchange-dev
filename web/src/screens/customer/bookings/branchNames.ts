// C03: the customer's branch names for My Bookings (design customer-app.md §4.5). A booking carries
// `branchId`, and a branch's public `code` is its id (`branches.code == branches.id`), so the public branch
// list names it. SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the signature stays.

import type { PublicBranchesData } from '../../../shared/api/public/public_branches';

/** The fetch My Bookings already uses. */
export type BranchNamesFetch = (input: string, init?: RequestInit) => Promise<Response>;

/**
 * Branch id → display name, from `GET /api/public/host` (`{org}`) then
 * `GET /api/public/orgs/{org}/branches` (`PublicBranchesData`). Never throws: any failure answers an empty map,
 * and the screen falls back to the raw id.
 */
export async function loadBranchNames(fetchFn: BranchNamesFetch): Promise<Map<string, string>> {
  try {
    const hostRes = await fetchFn('/api/public/host');
    if (!hostRes.ok) return new Map();
    const host = (await hostRes.json()) as { org?: string } | null;
    if (!host || typeof host.org !== 'string' || !host.org) return new Map();

    const listRes = await fetchFn(`/api/public/orgs/${encodeURIComponent(host.org)}/branches`);
    if (!listRes.ok) return new Map();
    const data = (await listRes.json()) as PublicBranchesData | null;
    const map = new Map<string, string>();
    for (const b of data?.branches ?? []) {
      map.set(b.code, b.name);
    }
    return map;
  } catch {
    return new Map();
  }
}
