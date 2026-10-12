// TD-3 #1291: the screen's address (design §3 `TransactionRoute`, D1).

const PATH = /^\/desk\/transactions\/([^/]+)$/;

/** The transaction id in `/desk/transactions/<id>`, or `null` for any other path. */
export function idFromPath(pathname: string): string | null {
  const m = PATH.exec(pathname);
  return m ? decodeURIComponent(m[1]!) : null;
}

/** The `branch` in `?branch=`, or `undefined` when absent or empty. */
export function branchFromSearch(search: string): string | undefined {
  const branch = new URLSearchParams(search).get('branch');
  return branch ? branch : undefined;
}
