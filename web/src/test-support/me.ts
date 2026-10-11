// OR-7.7 (#1030 §3.3, §7, ruling (b) issuecomment-6105567502): test fixtures for the Blueprint `me`.
// Written with the up-front tests by the test-writer claude/5d45b017. Test support only: nothing in
// the app imports it.
//
// The web tests used to build `Me` as `{ kind, id, role, branch, name, username, … }` with a v3 role.
// `meOf` moves such a fixture 1:1 to #1030's shape:
//   - the v3 role maps to Blueprint roles by design §3.3 (OWNER → EXECUTIVE + ADMIN, organisation-wide;
//     every other role at the fixture's branch);
//   - `permissions` is the union of `RolePermissions::of` over those roles, copied from
//     `api/crates/xc-auth/src/domain/access/role_permissions.rs` on main e6e6d22c, sorted;
//   - `kind` and `username` are gone.
// The cast keeps this file compiling against both the old and the new generated `Me`.
import type { Me } from '../shared/api/generated/common';
import { th } from '../shared/i18n/th';

export type MeRole = { role: string; branchId: string | null };

const PERMISSIONS: Record<string, readonly string[]> = {
  EXECUTIVE: [
    'OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'ApprovalsDecide', 'BookingsAdmin',
    'ReferenceRatesRead', 'SystemSettings', 'MarketingPerformance', 'AttendanceClock', 'CompatRead',
    'RequestAbandon', 'DocumentsList', 'FileRead', 'ProofUpload', 'DeviceQueue', 'DeviceProcess',
    'DevicePrinterOnline', 'DrawerPassport', 'SignedIn', 'ConsentPolicyManage',
  ],
  ADMIN: [
    'BranchesManage', 'UsersManage', 'SystemSettings', 'CashLimitsManage', 'AttendanceClock', 'CompatRead',
    'DocumentsList', 'SignedIn',
  ],
  RATE_MANAGER: ['RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'AttendanceClock', 'CompatRead', 'DocumentsList', 'SignedIn'],
  BRANCH_MANAGER: [
    'ShiftOperate', 'ShiftCloseCertify', 'TransactionRead', 'ReceiptReprint', 'ReversalRequest', 'ApprovalsDecide',
    'CashManage', 'QueueOperate', 'IdentityDocumentRead', 'CustomerHoldPlace', 'BranchDayOperate', 'AttendanceClock',
    'CompatRead', 'DocumentsList', 'FileRead', 'DeviceQueue', 'DeviceProcess', 'DevicePrinterOnline', 'DrawerPassport',
    'SignedIn',
  ],
  TELLER: [
    'ShiftOperate', 'BookingsDesk', 'ExchangeTransact', 'TransactionRead', 'ReceiptReprint', 'ReversalRequest',
    'QueueOperate', 'IdentityDocumentRead', 'CustomerHoldPlace', 'AttendanceClock', 'CompatRead', 'DocumentsList',
    'FileRead', 'ProofUpload', 'DeviceQueue', 'DeviceProcess', 'DrawerPassport', 'SignedIn',
  ],
  FINANCE: [
    'CashManage', 'FinanceReconcile', 'CashTargetSet', 'CashLimitsManage', 'ApprovalsDecide', 'IdentityDocumentRead',
    'AttendanceClock', 'CompatRead', 'DocumentsList', 'FileRead', 'ProofUpload', 'SignedIn',
  ],
  MARKETING: ['MarketingCampaigns', 'MarketingMembers', 'MarketingPerformance', 'AttendanceClock', 'SignedIn'],
  VERIFICATION_REVIEWER: ['VerificationReview', 'IdentityDocumentRead', 'AttendanceClock', 'CompatRead', 'DocumentsList', 'FileRead', 'SignedIn'],
  CUSTOMER: ['CustomerSelf', 'SignedIn'],
};

/** Design §3.3: a v3 role → its Blueprint roles. */
const MAPPING: Record<string, readonly string[]> = {
  OWNER: ['EXECUTIVE', 'ADMIN'],
  FRONT: ['TELLER'],
  BACKEND: ['BRANCH_MANAGER'],
  MANAGER: ['BRANCH_MANAGER'],
  RATES: ['RATE_MANAGER'],
  FINANCE: ['FINANCE'],
  SYSTEM: ['ADMIN'],
  MONITOR: ['VERIFICATION_REVIEWER'],
  DOCUMENTS: ['VERIFICATION_REVIEWER'],
  DOCUMENT_REVIEW: ['VERIFICATION_REVIEWER'],
  // fixtures already written with a Blueprint role keep it
  EXECUTIVE: ['EXECUTIVE'],
  ADMIN: ['ADMIN'],
  RATE_MANAGER: ['RATE_MANAGER'],
  BRANCH_MANAGER: ['BRANCH_MANAGER'],
  TELLER: ['TELLER'],
  MARKETING: ['MARKETING'],
  VERIFICATION_REVIEWER: ['VERIFICATION_REVIEWER'],
};

/** The Blueprint role rows of a v3 role held at `branch` (OWNER, or `orgWideRow`: organisation-wide). */
export function rolesOf(v3Role: string, branch: string, orgWideRow = false): MeRole[] {
  const mapped = MAPPING[v3Role];
  if (!mapped) throw new Error(`no §3.3 mapping for v3 role ${v3Role}`);
  const orgWide = orgWideRow || v3Role === 'OWNER';
  return mapped
    .map((role) => ({ role, branchId: orgWide ? null : branch }))
    .sort((a, b) => a.role.localeCompare(b.role) || String(a.branchId).localeCompare(String(b.branchId)));
}

/** The sorted union of the roles' permissions. */
export function permissionsOf(roles: readonly MeRole[]): string[] {
  const all = new Set<string>();
  for (const r of roles) for (const p of PERMISSIONS[r.role] ?? []) all.add(p);
  return [...all].sort();
}

export interface OldMe {
  id: string;
  role: string;
  branch: string;
  name: string | null;
  branchName?: string;
  branches?: { id: string; code: string; name: string }[];
  /** The role row is organisation-wide (`branchId: null`), as an org ADMIN's is. */
  orgWide?: boolean;
}

/** A v3 `me` fixture moved 1:1 to #1030's `Me`. */
export function meOf(old: OldMe): Me {
  const roles = rolesOf(old.role, old.branch, old.orgWide);
  return {
    id: old.id,
    name: old.name,
    roles,
    permissions: permissionsOf(roles),
    branch: old.branch,
    branchName: old.branchName ?? old.branch,
    branches: old.branches ?? [],
  } as unknown as Me;
}

/** `kiosk_notifications.employee` for the same caller (#1030 §3.4: `{ id, roles }`, as `me` fills them). */
export function employeeOf(me: Me): { id: string; roles: MeRole[] } {
  const m = me as unknown as { id: string; roles: MeRole[] };
  return { id: m.id, roles: m.roles };
}

/** The shared table's label of a role (`shell.account.role.<camelCase>`, ruling (a) on #1030), whatever its wording. */
export function roleLabel(camel: string, table: Record<string, string> = th as unknown as Record<string, string>): string {
  return table[`shell.account.role.${camel}`] as string;
}
