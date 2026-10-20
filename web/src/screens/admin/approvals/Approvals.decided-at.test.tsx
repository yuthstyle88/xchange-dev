// #2507: Admin Approvals decided-at timestamp renders in Asia/Bangkok date + time
// in the screen's locale while keeping raw server string on <time dateTime={...}>.

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { locale, setLang } from '../../../shared/i18n';
import { warmModules } from '../../../shared/testing/warm-modules';
import {
  makeApprovalDecisionDto,
  makeApprovalRequestItemDto,
  type ApprovalDecisionDto,
  type ApprovalRequestItemDto,
} from '../../../shared/testing/fixtures';
import { Approvals } from './Approvals';
import { en as screenEn, th as screenTh } from './i18n';
import { Toast, toast } from '../parts/toast';

beforeAll(() => warmModules(() => import('./Approvals')), 60_000);

const REQUESTS_PATH = '/api/v3/views/approval_requests';
const DECIDED_AT_UTC = '2026-10-15T18:30:00+00:00';

function makeRequest(
  id: string,
  status: 'PENDING' | 'APPROVED' | 'REJECTED',
  decision: ApprovalDecisionDto | null,
  branch = 'MAIN',
) {
  return makeApprovalRequestItemDto({
    id,
    type: 'RATE_PUBLICATION',
    branchId: branch,
    objectId: `draft-${id}`,
    objectVersionHash: `hash-${id}`,
    policyVersion: 'pol-1',
    proposedPayload: {
      draft_id: `draft-${id}`,
      policy_version_id: 'pol-1',
      reference_line_ids: ['line-1'],
      is_in_bound: false,
      out_of_bound: [{ currency: 'USD', side: 'SELL' }],
    },
    requestedBy: 'proposer-1',
    requestedAt: '2026-10-15T09:00:00Z',
    status,
    decision,
  });
}

describe('108-Plaza/xchange#2507: Admin Approvals decided-at Bangkok date and time (§8, §9)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let itemsByStatus: Record<string, ApprovalRequestItemDto[]>;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    setLang('th');
    itemsByStatus = {
      PENDING: [],
      APPROVED: [
        makeRequest(
          'req-approved-1',
          'APPROVED',
          makeApprovalDecisionDto({
            approverId: 'u-9',
            outcome: 'APPROVED',
            reason: 'ok',
            at: DECIDED_AT_UTC,
          }),
        ),
      ],
      REJECTED: [],
    };

    root = document.createElement('div');
    document.body.appendChild(root);
    toast('');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: unknown, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = (init?.method ?? 'GET').toUpperCase();

        if (url.pathname === REQUESTS_PATH && method === 'GET') {
          const status = url.searchParams.get('status') ?? '';
          const list = status
            ? (itemsByStatus[status] ?? [])
            : Object.values(itemsByStatus).flat();
          return Promise.resolve(
            new Response(
              JSON.stringify({
                version: 1,
                changed: true,
                data: { requests: structuredClone(list) },
              }),
              {
                status: 200,
                headers: { 'content-type': 'application/json' },
              },
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
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
    setLang('th');
  });

  function mount(): HTMLElement {
    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-15', to: '2026-10-15' });
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

  function normText(el: Element | null | undefined): string {
    return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
  }

  function switchButton(label: string): HTMLButtonElement {
    const switchGroup = root.querySelector('[data-status-switch]') ?? root;
    const found = [...switchGroup.querySelectorAll<HTMLButtonElement>('button')].filter(
      (b) => normText(b) === label,
    );
    expect(found.length, `status switch button ${JSON.stringify(label)}`).toBe(1);
    return found[0]!;
  }

  it('Row 1 (th & en): decision at 2026-10-15T18:30:00+00:00 renders visible text as 16 Oct 01:30 in Asia/Bangkok in the screen locale', async () => {
    setLang('th');
    mount();

    switchButton(screenTh.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-decision="req-approved-1"] time')).not.toBeNull();
    });

    const timeEl = root.querySelector('[data-decision="req-approved-1"] time')!;
    const expectedTh = new Date(DECIDED_AT_UTC).toLocaleString('th-TH', {
      timeZone: 'Asia/Bangkok',
    });
    expect(timeEl.textContent).toBe(expectedTh);
    expect(timeEl.textContent).toContain('16/10/2569');
    expect(timeEl.textContent).toContain('01:30');
    expect(timeEl.textContent).not.toContain('2026-10-15');
    expect(timeEl.textContent).not.toContain('18:30');

    setLang('en');
    await vi.waitFor(() => {
      const expectedEn = new Date(DECIDED_AT_UTC).toLocaleString('en-GB', {
        timeZone: 'Asia/Bangkok',
      });
      expect(timeEl.textContent).toBe(expectedEn);
    });
    expect(timeEl.textContent).toContain('16/10/2026');
    expect(timeEl.textContent).toContain('01:30');
    expect(timeEl.textContent).not.toContain('2026-10-15');
    expect(timeEl.textContent).not.toContain('18:30');
  });

  it('Row 2: keeps the raw server string on the <time> dateTime attribute while formatting visible text in Bangkok time', async () => {
    setLang('en');
    mount();

    switchButton(screenEn.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-decision="req-approved-1"] time')).not.toBeNull();
    });

    const timeEl = root.querySelector('[data-decision="req-approved-1"] time')!;
    expect(timeEl.getAttribute('dateTime')).toBe(DECIDED_AT_UTC);
    expect(timeEl.textContent).toBe(
      new Date(DECIDED_AT_UTC).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' }),
    );
    expect(timeEl.textContent).not.toBe(DECIDED_AT_UTC);
  });

  it('Edge case: unparsable decision timestamp shows raw string (never "Invalid Date") while valid timestamp formats in Asia/Bangkok', async () => {
    itemsByStatus.APPROVED = [
      makeRequest(
        'req-bad-date',
        'APPROVED',
        makeApprovalDecisionDto({
          approverId: 'u-1',
          outcome: 'APPROVED',
          reason: 'legacy timestamp',
          at: 'not-a-valid-date',
        }),
      ),
      makeRequest(
        'req-good-date',
        'APPROVED',
        makeApprovalDecisionDto({
          approverId: 'u-2',
          outcome: 'APPROVED',
          reason: 'valid timestamp',
          at: DECIDED_AT_UTC,
        }),
      ),
    ];

    setLang('en');
    mount();

    switchButton(screenEn.approved).click();

    await vi.waitFor(() => {
      expect(root.querySelector('[data-decision="req-bad-date"] time')).not.toBeNull();
      expect(root.querySelector('[data-decision="req-good-date"] time')).not.toBeNull();
    });

    const badTimeEl = root.querySelector('[data-decision="req-bad-date"] time')!;
    expect(badTimeEl.getAttribute('dateTime')).toBe('not-a-valid-date');
    expect(badTimeEl.textContent).toBe('not-a-valid-date');
    expect(root.textContent).not.toContain('Invalid Date');

    const goodTimeEl = root.querySelector('[data-decision="req-good-date"] time')!;
    expect(goodTimeEl.getAttribute('dateTime')).toBe(DECIDED_AT_UTC);
    expect(goodTimeEl.textContent).toBe(
      new Date(DECIDED_AT_UTC).toLocaleString('en-GB', { timeZone: 'Asia/Bangkok' }),
    );
  });
});
