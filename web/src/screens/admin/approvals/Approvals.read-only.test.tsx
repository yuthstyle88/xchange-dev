// R4 / F37 (#564): a listed request the reader may not decide is shown read-only.
// Written before the implementation by the ticket author (claude/33691c06). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Owner ruling R4 (B) on #564, issuecomment-6673699296 (accepted 2026-11-06 07:56 ICT):
// "FINANCE and BRANCH_MANAGER see over-limit rows read-only, with no decide button."
//
// The API now lists an over-limit CASH_TRANSFER to FINANCE and BRANCH_MANAGER with
// `canDecide: false` (pinned in api/tests/f37_transfer_approvals_visible_read_only_test.rs).
// Pinned here:
// - the decide form (reason input, Reject, Approve) renders only when the request is not decided
//   AND `request.canDecide` is true;
// - a pending row with `canDecide: false` shows the line `รอผู้มีสิทธิ์อนุมัติ`
//   (en `Waiting for an approver with the limit`) instead, and keeps its detail lines;
// - a decided row still shows its decision record, whatever `canDecide` says;
// - the generated fixture `makeApprovalRequestItemDto()` defaults to `canDecide: true`, so every
//   test written before R4 still builds a row its reader can decide.
//
// Rows get `canDecide` through a cast so this file type-checks before the generated type has it.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { th, en } from './i18n';
import { Approvals } from './Approvals';
import { Toast } from '../parts/toast';
import { makeApprovalRequestItemDto } from '../../../shared/testing/fixtures';

const REQUESTS_PATH = '/api/v3/views/approval_requests';
const WAITING_TH = 'รอผู้มีสิทธิ์อนุมัติ';
const WAITING_EN = 'Waiting for an approver with the limit';

type Row = ReturnType<typeof makeApprovalRequestItemDto>;

function transferRow(
  id: string,
  canDecide: boolean,
  withinLimit = false,
  status = 'PENDING',
): Row {
  const row = makeApprovalRequestItemDto({
    id,
    type: 'CASH_TRANSFER',
    branchId: 'MAIN',
    objectId: `obj-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: '1',
    proposedPayload: {
      transfer_id: '74f7ecfe-cc4a-4b94-aac2-7b32f08ff33e',
      from_branch: 'MAIN',
      within_limit: withinLimit,
    },
    requestedBy: '43839561-1111-4000-8000-000000000001',
    requestedAt: '2026-11-05T03:00:00Z',
    status,
    decision: null,
  });
  return { ...row, canDecide } as Row;
}

function buttons(card: Element, label: string): Element[] {
  return Array.from(card.querySelectorAll('button')).filter(
    (b) => (b.textContent ?? '').trim() === label,
  );
}

describe('R4/F37 Approvals: a row the reader may not decide is read-only', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let currentRequests: Row[];

  beforeEach(() => {
    setLang('th');
    currentRequests = [];
    root = document.createElement('div');
    document.body.appendChild(root);

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();
        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: structuredClone(currentRequests) },
              }),
              { status: 200, headers: { 'content-type': 'application/json' } },
            ),
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

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-11-05', to: '2026-11-05' });
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
    return root;
  }

  async function card(id: string): Promise<Element> {
    await vi.waitFor(() => {
      expect(root.querySelector(`[data-request="${id}"]`)).not.toBeNull();
    });
    return root.querySelector(`[data-request="${id}"]`)!;
  }

  it('the generated fixture defaults to canDecide: true', () => {
    expect((makeApprovalRequestItemDto() as unknown as { canDecide?: unknown }).canDecide).toBe(
      true,
    );
  });

  it('a pending row with canDecide true shows the decide form', async () => {
    currentRequests = [transferRow('can', true)];
    mount();
    const c = await card('can');
    expect(buttons(c, th.approve)).toHaveLength(1);
    expect(buttons(c, th.reject)).toHaveLength(1);
    expect(c.querySelector('input[type="text"]')).not.toBeNull();
    expect(c.textContent).not.toContain(WAITING_TH);
  });

  it('a pending over-limit row with canDecide false has no decide form and says who decides', async () => {
    currentRequests = [transferRow('cannot', false)];
    mount();
    const c = await card('cannot');
    await vi.waitFor(() => {
      expect(c.textContent).toContain(WAITING_TH);
    });
    expect(buttons(c, th.approve), 'no Approve button').toHaveLength(0);
    expect(buttons(c, th.reject), 'no Reject button').toHaveLength(0);
    expect(c.querySelector('input[type="text"]'), 'no reason input').toBeNull();
    // The detail line of #5314 is still there: the reader sees what is waiting.
    expect(c.textContent).toContain('#74f7ecfe · MAIN · เกินวงเงิน — ผู้บริหารตัดสิน');

    setLang('en');
    await vi.waitFor(() => {
      expect(c.textContent).toContain(WAITING_EN);
    });
    expect(buttons(c, en.approve)).toHaveLength(0);
    expect(buttons(c, en.reject)).toHaveLength(0);
  });

  it('a pending within-limit row with canDecide false is read-only too (FINANCE)', async () => {
    currentRequests = [transferRow('fin', false, true)];
    mount();
    const c = await card('fin');
    await vi.waitFor(() => {
      expect(c.textContent).toContain(WAITING_TH);
    });
    expect(buttons(c, th.approve)).toHaveLength(0);
    expect(buttons(c, th.reject)).toHaveLength(0);
  });

  it('only the row the reader may not decide loses its form', async () => {
    currentRequests = [transferRow('ro', false), transferRow('rw', true, true)];
    mount();
    const ro = await card('ro');
    const rw = await card('rw');
    await vi.waitFor(() => {
      expect(ro.textContent).toContain(WAITING_TH);
    });
    expect(buttons(ro, th.approve)).toHaveLength(0);
    expect(buttons(rw, th.approve)).toHaveLength(1);
    expect(rw.textContent).not.toContain(WAITING_TH);
  });

  it('a decided row shows its decision record, not the waiting line, whatever canDecide says', async () => {
    const decided = {
      ...transferRow('done', false, false, 'APPROVED'),
      decision: {
        approverId: 'owner-1',
        approverName: 'Owner',
        outcome: 'APPROVED',
        reason: 'ok',
        at: '2026-11-05T04:00:00Z',
      },
    } as Row;
    currentRequests = [decided];
    mount();
    const c = await card('done');
    await vi.waitFor(() => {
      expect(c.querySelector('[data-decision="done"]')).not.toBeNull();
    });
    expect(c.textContent).not.toContain(WAITING_TH);
    expect(buttons(c, th.approve)).toHaveLength(0);
  });
});
