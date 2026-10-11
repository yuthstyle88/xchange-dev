// RD-3 location helper (design rate-display.md §3, D2).
export interface DisplayLocation {
  org: string;
  branch: string;
}

/** The `branch` of `?branch=:code`, or `null` when there is none. */
export function branchFromSearch(search: string): string | null {
  const params = new URLSearchParams(search);
  const branch = params.get('branch');
  return branch && branch.trim().length > 0 ? branch.trim() : null;
}

export function withOrg(branch: string, org: string): DisplayLocation {
  return { org, branch };
}
