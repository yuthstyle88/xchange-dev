// G1-FE #5925: a decided approval card names the approver instead of "—" (staff rerun 2026-11-02, G1 on #564).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/staff-e2e-1101-reads.md` T3 §2 (amended by PR #5913): the decision carries
// `approverName` = the approver's `users.name`, or null for a blank name, an id with no users row, or a
// policy decision (`approver_id` = `policy:<version>`, `api/crates/xc-approval/src/application/approval_service.rs:337`).
// The fallback (FE's choice, agreed with the BE sub-controller claude/b091f5e0): the name; else, for a
// `policy:` id, the policy word (a new key `decidedByPolicy`); else "—". The id stays only in `title` (W13).
// One test per row of the ticket's behaviour table; the row id is in the test name.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import * as table from './i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import { makeApprovalDecisionDto, makeApprovalRequestItemDto } from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';

// The ticket's wording (§7). A new key of `./i18n.ts`.
const POLICY_WORD = { th: 'อนุมัติอัตโนมัติตามนโยบาย', en: 'Automatic, by policy' } as const;

const APPROVER = '9a0b1c2d-2222-4000-8000-000000000002';
const POLICY_ID = 'policy:pol-7';
const UUID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

function decided(id: string, decision: ReturnType<typeof makeApprovalDecisionDto>) {
  return makeApprovalRequestItemDto({
    id,
    type: 'TRANSACTION_REVERSAL',
    branchId: 'MAIN',
    objectId: `obj-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-7',
    proposedPayload: { reason: 'x', amountThb: '100.00' },
    requestedBy: 'u-req',
    requestedByName: 'Nok',
    requestedAt: '2026-11-01T03:00:00Z',
    status: 'APPROVED',
    decision,
  });
}

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

describe('G1-FE: the decided card names the approver (T3 §2)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let approved: ReturnType<typeof makeApprovalRequestItemDto>[];

  beforeEach(() => {
    setLang('th');
    approved = [];
    root = document.createElement('div');
    document.body.appendChild(root);
    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          const rows = url.searchParams.get('status') === 'APPROVED' ? approved : [];
          return Promise.resolve(
            new Response(JSON.stringify({ version: 1, changed: true, data: { requests: structuredClone(rows) } }), {
              status: 200,
              headers: { 'content-type': 'application/json' },
            }),
          );
        }
        return Promise.resolve(
          new Response(JSON.stringify({ version: 1, changed: false }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          }),
        );
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    root?.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    setLang('th');
  });

  /** Mounts the screen, opens the Approved tab and returns the decision block of request `id`. */
  async function decisionOf(id: string): Promise<HTMLElement> {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-01', to: '2026-11-01' });
    const [shown] = createSignal(true);
    dispose = render(
      () =>
        (
          <>
            <Approvals branch={branch} period={period} shown={shown} />
            <Toast />
          </>
        ) as JSX.Element,
      root,
    );
    await vi.waitFor(() => {
      expect(root.querySelector('[data-status-filter="APPROVED"]')).not.toBeNull();
    });
    (root.querySelector('[data-status-filter="APPROVED"]') as HTMLButtonElement).click();
    await vi.waitFor(() => {
      expect(root.querySelector(`[data-decision="${id}"]`), `decision ${id}`).not.toBeNull();
    });
    return root.querySelector(`[data-decision="${id}"]`) as HTMLElement;
  }

  it('A0: the key decidedByPolicy exists in th and en with the ticket wording', () => {
    for (const lang of ['th', 'en'] as const) {
      expect((table[lang] as Record<string, string>).decidedByPolicy, `admin.approvals ${lang}.decidedByPolicy`).toBe(POLICY_WORD[lang]);
    }
  });

  it('A1: a named approver shows the name; the id is only in title', async () => {
    approved = [decided('d-1', makeApprovalDecisionDto({ approverId: APPROVER, approverName: 'Somsri', outcome: 'APPROVED', reason: 'ok' }))];
    const block = await decisionOf('d-1');
    const who = block.querySelector(`[title="${APPROVER}"]`);
    expect(who, 'the approver id is a title').not.toBeNull();
    expect(text(who)).toBe('Somsri');
    expect(text(block)).toContain(`${(table.th as Record<string, string>).decidedBy.trim()} Somsri`);
    expect(text(block)).not.toMatch(UUID);
  });

  it('A2: approverName null with a user id shows "—"; the id is only in title', async () => {
    approved = [decided('d-2', makeApprovalDecisionDto({ approverId: APPROVER, approverName: null, outcome: 'APPROVED', reason: 'ok' }))];
    const block = await decisionOf('d-2');
    const who = block.querySelector(`[title="${APPROVER}"]`);
    expect(who).not.toBeNull();
    expect(text(who)).toBe('—');
    expect(text(block)).not.toMatch(UUID);
  });

  it('A3: approverName null with a policy id shows the policy word; the id is only in title', async () => {
    approved = [decided('d-3', makeApprovalDecisionDto({ approverId: POLICY_ID, approverName: null, outcome: 'APPROVED', reason: 'policy' }))];
    const block = await decisionOf('d-3');
    const who = block.querySelector(`[title="${POLICY_ID}"]`);
    expect(who, 'the policy id is a title').not.toBeNull();
    expect(text(who)).toBe(POLICY_WORD.th);
    expect(text(block)).not.toContain('policy:');
  });

  it('A4: the policy word follows setLang (en)', async () => {
    setLang('en');
    approved = [decided('d-4', makeApprovalDecisionDto({ approverId: POLICY_ID, approverName: null, outcome: 'APPROVED', reason: 'policy' }))];
    const block = await decisionOf('d-4');
    expect(text(block.querySelector(`[title="${POLICY_ID}"]`))).toBe(POLICY_WORD.en);
  });

  it('A5: a name wins over the policy rule (the name is shown whenever it is sent)', async () => {
    approved = [decided('d-5', makeApprovalDecisionDto({ approverId: POLICY_ID, approverName: 'Named', outcome: 'APPROVED', reason: 'x' }))];
    const block = await decisionOf('d-5');
    expect(text(block.querySelector(`[title="${POLICY_ID}"]`))).toBe('Named');
  });
});
