// DeskPath: the one place /desk and /staff/desk differ (design xchange-v3-wave3-desk.md §5.2, D-11).
// Route values are the server's: rpc.rs:94-95, files.rs:32,37, bookings.rs:78,84, compat/staff_view.rs:19,21.

/** The Blueprint quote and transaction routes (quote-transaction.md §4.2, §10). Both paths reach the same seam today. */
export interface QuoteRoutes {
  quotes: '/api/quotes';
  transactions: '/api/transactions';
  /** The transaction view re-read while an exchange waits for approval. */
  transactionView: 'transaction';
}

export interface DeskPath {
  kind: 'owner' | 'staff';
  configView: 'desk_config' | 'staff_desk_config';
  quoteRoutes: QuoteRoutes;
  rpc: '/api/rpc' | '/api/staff/rpc';
  upload: '/api/upload' | '/api/staff/upload';
  bookings: '/api/bookings' | '/api/staff/bookings';
  /** '/files/<key>' | '/api/staff/file/<key>' (pages.rs:429-432). */
  fileUrl(key: string): string;
  /** `ดูงานพิมพ์และบันทึกหลังบ้าน →` on /desk only (desk-devices.js:5). */
  devicesLink: boolean;
  /** Staff: opsBeginTransaction before the first reserveCustomer (X5). */
  usesDraft: boolean;
}

const QUOTE_ROUTES: QuoteRoutes = {
  quotes: '/api/quotes',
  transactions: '/api/transactions',
  transactionView: 'transaction',
};

export const OWNER_DESK: DeskPath = {
  kind: 'owner',
  configView: 'desk_config',
  quoteRoutes: QUOTE_ROUTES,
  rpc: '/api/rpc',
  upload: '/api/upload',
  bookings: '/api/bookings',
  fileUrl: (key) => `/files/${key}`,
  devicesLink: true,
  usesDraft: false,
};

export const STAFF_DESK: DeskPath = {
  kind: 'staff',
  configView: 'staff_desk_config',
  quoteRoutes: QUOTE_ROUTES,
  rpc: '/api/staff/rpc',
  upload: '/api/staff/upload',
  bookings: '/api/staff/bookings',
  fileUrl: (key) => `/api/staff/file/${key}`,
  devicesLink: false,
  usesDraft: true,
};

/** '/staff/desk' exactly is the staff desk; every other pathname is the owner's. */
export function deskPathFor(pathname: string): DeskPath {
  return pathname === '/staff/desk' ? STAFF_DESK : OWNER_DESK;
}
