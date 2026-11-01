// FE-T6 #5854: the verification screen shows its times in ICT (`dd/MM/yyyy HH:mm`).
// Written before the implementation by claude/01TxaRJG (FE sub-controller). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/fe-triage-1101.md` W1 (PR #5848). The three raw times on main `a174fb95`:
// the queue's `submittedAt` (`VerificationScreen.tsx:795`), the case's `expiresAt` (`:835`) and the
// audit line's timestamp (`renderAuditSummary`, `:721`). Harness shape follows
// `Verification.case-order.test.tsx`. One test per row; the row id is in the test name.
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setLang } from '../../../shared/i18n';
import { meOf } from '../../../test-support/me';
import { VerificationScreen } from './VerificationScreen';

const CASE_ID = '11111111-1111-1111-1111-111111111111';
const ISO = /\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/;

const REVIEWER = meOf({
  id: '00000000-0000-4000-8000-000000000001',
  role: 'VERIFICATION_REVIEWER',
  branch: 'MAIN',
  name: 'Reviewer Somchai',
  branchName: 'Head Office',
});

const QUEUE = [
  {
    id: CASE_ID,
    status: 'SUBMITTED',
    submittedAt: '2026-10-31T17:30:00Z', // 01/11/2026 00:30
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
];

const CASE = {
  case: {
    id: CASE_ID,
    status: 'APPROVED',
    version: 4,
    submittedAt: '2026-10-31T17:30:00Z',
    expiresAt: '2027-04-30T17:00:00+00:00', // 01/05/2027 00:00
    customerDisplayName: 'Somchai Jaidee',
    identityLast4: '1234',
    branchId: 'MAIN',
  },
  documents: [],
  audit: [
    {
      actorId: '00000000-0000-4000-8000-000000000091',
      role: 'VERIFICATION_REVIEWER',
      action: 'verification_case.approve',
      objectRef: CASE_ID,
      beforeState: { status: 'UNDER_REVIEW' },
      afterState: { status: 'APPROVED' },
      createdAt: '2026-11-01T05:44:25.455952232+00:00', // 01/11/2026 12:44
    },
  ],
};

function json(data: unknown, status = 200): Response {
  return new Response(JSON.stringify(data), { status, headers: { 'Content-Type': 'application/json' } });
}

describe('FE-T6 #5854: verification times in ICT (design W1)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | null = null;

  async function settle(): Promise<void> {
    for (let i = 0; i < 15; i++) await vi.advanceTimersByTimeAsync(0);
  }

  beforeEach(() => {
    vi.useFakeTimers();
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      if (u.pathname === '/api/v3/views/me' || u.pathname === '/api/v3/views/staff_me') {
        return json({ version: 1, changed: true, data: REVIEWER });
      }
      if (u.pathname === '/api/v3/views/verification_policy') {
        return json({ version: 1, changed: true, data: { branchReviewAllowed: false } });
      }
      if (u.pathname === '/api/v3/views/verification_queue') {
        return json({ version: 10, changed: true, data: QUEUE });
      }
      if (u.pathname === '/api/v3/views/verification_case') {
        return json({ version: CASE.case.version, changed: true, data: CASE });
      }
      return json({ version: 1, changed: false });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    host.remove();
    vi.unstubAllGlobals();
    vi.useRealTimers();
    setLang('th');
  });

  async function open(): Promise<void> {
    dispose = render(() => <VerificationScreen />, host);
    await settle();
    const row = host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`);
    expect(row, 'the queue row').not.toBeNull();
  }

  it('V1: the queue row shows submittedAt in ICT, rolled to the Bangkok day', async () => {
    await open();
    const row = host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`)!;
    expect(row.textContent).toContain('01/11/2026 00:30');
    expect(row.textContent ?? '').not.toMatch(ISO);
  });

  it('V2: the case detail shows expiresAt in ICT', async () => {
    await open();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`)!.click();
    await settle();
    const detail = host.querySelector('[data-testid="case-detail-view"]');
    expect(detail, 'the case detail').not.toBeNull();
    expect(detail!.textContent).toContain('01/05/2027 00:00');
  });

  it('V3: the audit line shows its time in ICT', async () => {
    await open();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`)!.click();
    await settle();
    const audit = host.querySelector('[data-testid="case-audit-list"]');
    expect(audit, 'the audit list').not.toBeNull();
    expect(audit!.textContent).toContain('01/11/2026 12:44');
    expect(audit!.textContent ?? '').not.toMatch(ISO);
  });

  it('V4: no ISO time is left on the opened case', async () => {
    await open();
    host.querySelector<HTMLElement>(`[data-case-id="${CASE_ID}"]`)!.click();
    await settle();
    expect(host.textContent ?? '').not.toMatch(ISO);
  });
});
