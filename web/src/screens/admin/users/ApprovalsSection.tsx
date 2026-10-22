// AP-F1 skeleton (XC FE lane controller claude/e8020026, 2026-10-22). Design: docs/design/blueprint/user-management-rbac.md
// §11.9 "FE" + guardrails (rule 4, new rule 6). Up-front tests: PermissionMatrix.approvals.test.tsx (do not edit them).
//
// The permission matrix (F5, `rbac_f12`) gains an "Approvals" section: the nine approval-kind permissions, on/off per
// group. The worker fills in the two components below and wires them into PermissionMatrix.tsx.
//
// DOM contract (the tests query exactly these):
//
// ApprovalsOverview — read-only, rendered under the module table for every caller:
//   <section data-matrix-approvals>            heading text t('matrixApprovalsTitle')
//     one row per kind, in APPROVAL_KINDS order: [data-matrix-approval-row="<Kind>"], its label t(`matrixApproval_<Kind>`)
//     one cell per group per kind, groups in the view's order: [data-matrix-approval-cell="<code>:<Kind>"] with
//       data-held="true"  and text t('matrixApprovalHeld')     when group.permissions holds the kind,
//       data-held="false" and text t('matrixApprovalNotHeld')  when it does not,
//       data-held="false" and text t('matrixApprovalUnavailable') for a group whose base is ADMIN (§11.9 rule 6).
//     It holds no <input>: nothing here is editable (the built-in EXECUTIVE group is fixed, rule 4, and an edit happens
//     only in the edit panel, which PermissionMatrix never opens for a group the view marks `editable: false`).
//
// ApprovalsEditor — inside PermissionMatrix's edit panel, for the group being edited:
//   <div data-matrix-approvals-edit>           heading text t('matrixApprovalsTitle')
//     nine <input type="checkbox" data-matrix-approval="<Kind>">, in APPROVAL_KINDS order, each in a <label> whose
//       text is t(`matrixApproval_<Kind>`); checked iff props.selected has the kind; a change calls props.onToggle.
//     base ADMIN: all nine disabled and unchecked, plus [data-matrix-approvals-unavailable] with
//       t('matrixApprovalsAdminReason').
//
// PermissionMatrix.tsx (worker):
//   - the edit selection starts with the editable module permissions AND the approval kinds the group holds
//     (none for an ADMIN base);
//   - the save sends `permissions` = the selected module permissions and approval kinds, in the view's `permissions`
//     order. Never `ApprovalsDecide` (the server sets it, §11.9 A8) and never another no-module permission. An ADMIN
//     base never sends a kind;
//   - `409 guardrail:6` is shown as t('matrixGuardrail_6') in the existing role="alert" [data-matrix-error], and the
//     edit stays open with the user's choices.
//
// The view lists the nine with `module: null, editable: false` (they are no-module, §11.9), so the screen recognises
// them by name through APPROVAL_KINDS, not by the view's `editable` flag.

import { For, Show, type JSX } from 'solid-js';
import type { MatrixGroup } from '../../../shared/api/generated/admin_permission_matrix';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

const t = screenT('admin.users', table);

/** The nine approval-kind permissions (serde names), in `Permission::ALL` / `APPROVAL_KINDS` order (§11.9). */
export const APPROVAL_KINDS = [
  'ApproveCashTransferWithinLimit',
  'ApproveCashTransferAboveLimit',
  'ApproveCountDifference',
  'ApproveNegativeStock',
  'ApproveCampaignVersion',
  'ApproveRatePublication',
  'ApproveExchangeTransaction',
  'ApproveTransactionReversal',
  'ApproveLoyaltyRuleVersion',
] as const;

export type ApprovalKind = (typeof APPROVAL_KINDS)[number];

const KINDS: ReadonlySet<string> = new Set<string>(APPROVAL_KINDS);

export function isApprovalKind(name: string): name is ApprovalKind {
  return KINDS.has(name);
}

/** The base whose groups hold none of the nine (§11.9 rule 6). */
export const ADMIN_BASE = 'ADMIN';

export interface ApprovalsOverviewProps {
  /** The view's groups, in the view's order. */
  groups: MatrixGroup[];
}

/** Read-only: the nine kinds as rows, one on/off cell per group. */
export function ApprovalsOverview(props: ApprovalsOverviewProps): JSX.Element {
  return (
    <section class="users-matrix-panel" data-matrix-approvals="">
      <h4 class="users-dialog-title">{t('matrixApprovalsTitle')}</h4>
      <div class="users-table-scroll">
        <table class="users-table">
          <thead>
            <tr>
              <th class="users-th">{t('matrixApprovalsTitle')}</th>
              <For each={props.groups}>
                {(group) => <th class="users-th">{group.name}</th>}
              </For>
            </tr>
          </thead>
          <tbody>
            <For each={APPROVAL_KINDS}>
              {(kind) => (
                <tr data-matrix-approval-row={kind}>
                  <td class="users-td">{t(`matrixApproval_${kind}`)}</td>
                  <For each={props.groups}>
                    {(group) => {
                      const unavail = group.base === ADMIN_BASE;
                      const held = !unavail && group.permissions.includes(kind);
                      return (
                        <td
                          class="users-td"
                          data-matrix-approval-cell={`${group.code}:${kind}`}
                          data-held={held ? 'true' : 'false'}
                        >
                          {unavail
                            ? t('matrixApprovalUnavailable')
                            : held
                              ? t('matrixApprovalHeld')
                              : t('matrixApprovalNotHeld')}
                        </td>
                      );
                    }}
                  </For>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
}

export interface ApprovalsEditorProps {
  /** The edited group's base role code; `ADMIN` disables all nine (rule 6). */
  base: string;
  /** The edit panel's current selection (module permissions and approval kinds together). */
  selected: Set<string>;
  /** Called when the user switches a kind on or off. */
  onToggle: (kind: ApprovalKind, on: boolean) => void;
}

/** The nine on/off toggles for the group being edited. */
export function ApprovalsEditor(props: ApprovalsEditorProps): JSX.Element {
  const isAdmin = () => props.base === ADMIN_BASE;
  return (
    <div class="users-field" data-matrix-approvals-edit="">
      <span class="users-label">{t('matrixApprovalsTitle')}</span>
      <Show when={isAdmin()}>
        <p class="users-intro" data-matrix-approvals-unavailable="">
          {t('matrixApprovalsAdminReason')}
        </p>
      </Show>
      <div class="users-matrix-perms">
        <For each={APPROVAL_KINDS}>
          {(kind) => (
            <label class="users-matrix-perm">
              <input
                type="checkbox"
                data-matrix-approval={kind}
                disabled={isAdmin()}
                checked={!isAdmin() && props.selected.has(kind)}
                onChange={(e) => {
                  if (!isAdmin()) {
                    props.onToggle(kind, e.currentTarget.checked);
                  }
                }}
              />
              <span>{t(`matrixApproval_${kind}`)}</span>
            </label>
          )}
        </For>
      </div>
    </div>
  );
}
