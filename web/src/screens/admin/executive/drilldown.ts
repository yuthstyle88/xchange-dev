// EX-4 (#1186 §7): where a drill-down opens. EX-3 names a target view; A01 opens the admin menu that shows it,
// with the drill-down's filter as the hash query the shell applies (design §4.4).

import type { DrilldownDto } from '../../../shared/api/generated/executive_overview';

export const TARGETS: Record<string, string> = {
  admin_transactions: 'transactions',
  admin_customers: 'customers',
  approval_requests: 'approvals',
  admin_cash: 'cash',
  admin_overview: 'overview',
};

/** The admin menu that shows `view`, or `null` when no menu does (no link is shown). */
export function targetMenu(view: string): string | null {
  return Object.prototype.hasOwnProperty.call(TARGETS, view) ? TARGETS[view]! : null;
}

/** `#<menu>?<filter>`, or `null` when the view has no menu. An empty filter is the menu alone. */
export function drilldownHash(d: DrilldownDto): string | null {
  const menu = targetMenu(d.view);
  if (menu === null) return null;
  const query = new URLSearchParams(d.filter).toString();
  return query === '' ? `#${menu}` : `#${menu}?${query}`;
}
