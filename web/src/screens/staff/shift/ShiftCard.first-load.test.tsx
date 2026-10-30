// #5591 (DESK-R4 T5, #5582 item 4): the shift card draws neither count button until its first `cash_counts` read
// has answered, so a teller cannot start a second count while one is already WAITING.
// Written before the implementation by the ticket author claude/studio-claude-15. Do not edit, delete or weaken;
// add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { ShiftCard } from './ShiftCard';
import type { StaffCtx } from '../cards';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000a1';
const SHIFT = 'shift-mine';
const CLOCKED_IN = '2026-10-11T01:00:00Z';

const me = () =>
  makeStaffMeView({
    userId: ME,
    name: 'Somchai',
    roles: [],
    branchId: BRANCH,
    branchName: 'XC-B',
    permissions: ['ShiftOperate'],
  });

function floor(withShift: boolean) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: CLOCKED_IN,
    myShift: withShift ? { id: SHIFT, openedAt: '2026-10-11T01:05:00Z' } : null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

const DENOMS = [makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' })];

const waitingClose = () =>
  makeCashCountRow({
    id: 'c-close-1',
    branchId: BRANCH,
    shiftId: SHIFT,
    kind: 'CLOSE',
    countedBy: ME,
    countedAt: '2026-10-11T09:30:00Z',
    status: 'WAITING',
    verifiedBy: null,
    verifiedAt: null,
  });

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let releaseCounts: (() => void) | undefined;

/** `cash_counts` is held until `releaseCounts()`; it then answers `answer`. */
function stub(answer: { status: number; rows: ReturnType<typeof makeCashCountRow>[] }) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      if ((init?.method ?? 'GET').toUpperCase() !== 'GET') return json(200, {});
      const view = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
      if (view === 'cash_counts') {
        await new Promise<void>((r) => (releaseCounts = r));
        if (answer.status !== 200) {
          return json(answer.status, { error: { code: 'internal', message: 'boom' } });
        }
        return json(200, { version: 1, changed: true, data: answer.rows });
      }
      if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
      if (view === 'handovers_pending') return json(200, { version: 1, changed: true, data: [] });
      if (view === 'handover_receivers') return json(200, { version: 1, changed: true, data: { rows: [] } });
      return json(404, { error: { code: 'notFound', message: 'not found' } });
    }),
  );
}

function mount(withShift: boolean) {
  const ctx: StaffCtx = { me: me(), floor: floor(withShift), refresh: vi.fn(async () => {}) };
  dispose = render(() => <ShiftCard ctx={ctx} />, host);
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

const has = (id: string) => host.querySelector(`[data-testid="${id}"]`) !== null;

describe('#5591 ShiftCard waits for the first cash_counts read before drawing a count button', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    releaseCounts = undefined;
  });

  afterEach(() => {
    releaseCounts?.();
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  it('G1 open shift, read in flight: no close-count button and no waiting line; handover stays', async () => {
    stub({ status: 200, rows: [waitingClose()] });
    mount(true);
    await settle();
    expect(releaseCounts).toBeDefined();
    expect(has('shift-close-count')).toBe(false);
    expect(has('shift-close-waiting')).toBe(false);
    expect(has('shift-handover')).toBe(true);
  });

  it('G2 no shift, read in flight: no open-count button and no waiting line', async () => {
    stub({ status: 200, rows: [] });
    mount(false);
    await settle();
    expect(releaseCounts).toBeDefined();
    expect(has('shift-open-count')).toBe(false);
    expect(has('shift-open-waiting')).toBe(false);
  });

  it('G3 the read answers a WAITING close count: the waiting line shows, never the button', async () => {
    stub({ status: 200, rows: [waitingClose()] });
    mount(true);
    await settle();
    releaseCounts!();
    await settle();
    expect(has('shift-close-waiting')).toBe(true);
    expect(has('shift-close-count')).toBe(false);
  });

  it('G4 the read answers no count: the button shows', async () => {
    stub({ status: 200, rows: [] });
    mount(false);
    await settle();
    releaseCounts!();
    await settle();
    expect(has('shift-open-count')).toBe(true);
  });

  it('G5 the read fails: the error line shows and the button comes back (as today after a failed read)', async () => {
    stub({ status: 500, rows: [] });
    mount(true);
    await settle();
    releaseCounts!();
    await settle();
    expect(has('shift-error')).toBe(true);
    expect(has('shift-close-count')).toBe(true);
  });
});
