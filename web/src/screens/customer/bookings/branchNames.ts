// C03: the customer's branch names for My Bookings (design customer-app.md §4.5). A booking carries
// `branchId`, and a branch's public `code` is its id (`branches.code == branches.id`), so the public branch
// list names it. SKELETON STUB (feat/fe-customer): the function ticket replaces this body; the signature stays.

/** The fetch My Bookings already uses. */
export type BranchNamesFetch = (input: string, init?: RequestInit) => Promise<Response>;

/**
 * Branch id → display name, from `GET /api/public/host` (`{org}`) then
 * `GET /api/public/orgs/{org}/branches` (`PublicBranchesData`). Never throws: any failure answers an empty map,
 * and the screen falls back to the raw id.
 */
export async function loadBranchNames(_fetchFn: BranchNamesFetch): Promise<Map<string, string>> {
  return new Map();
}
