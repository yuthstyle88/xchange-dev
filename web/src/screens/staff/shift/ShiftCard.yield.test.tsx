// #5133 S1–S4: ShiftCard holds a queue-card yield exactly while its count form or hand-over form is open, so the
// floating queue card folds out of the teller's way, and releases it on cancel and on unmount.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ShiftCard } from './ShiftCard';
import { setLang } from '../../../shared/i18n';
import { queueCardYielding } from '../../../shared/ui/queue-card-yield';
import type { StaffCtx } from '../cards';
import {
  makeDenominationRow,
  makeHandoverReceiverRow,
  makeMyShift,
  makeStaffFloorMyShift,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';

const ME = '00000000-0000-4000-8000-0000000000a1';
const OTHER = '00000000-0000-4000-8000-0000000000a2';
const SHIFT_ID = 'shift-open-001';
const DENOMS = [makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' })];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function floor(open: boolean) {
  return makeStaffFloorView({
    phase: 'OPEN',
    hours: null,
    myClockIn: '2026-10-11T01:00:00Z',
    myShift: open ? makeStaffFloorMyShift({ id: SHIFT_ID, openedAt: '2026-10-11T01:05:00Z' }) : null,
    cutoff: null,
    target: null,
    safe: null,
  });
}

describe('#5133 ShiftCard yields the queue card while a form is open', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    const views: Record<string, unknown> = {
      cash_counts: [],
      handovers_pending: [],
      denominations: { rows: DENOMS },
      handover_receivers: { rows: [makeHandoverReceiverRow({ userId: OTHER, name: 'Somchai' })] },
      my_shift: makeMyShift({ id: SHIFT_ID, status: 'OPEN', openedAt: '2026-10-11T01:05:00Z', balances: [] }),
    };
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        const path = url.split('?')[0]!;
        if ((init?.method ?? 'GET').toUpperCase() === 'GET' && path.startsWith('/api/v3/views/')) {
          const name = path.slice('/api/v3/views/'.length);
          return name in views
            ? json(200, { version: 1, changed: true, data: views[name] })
            : json(404, { error: { code: 'notFound', message: 'not found' } });
        }
        return json(200, {});
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  async function settle() {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
  }

  async function mount(open: boolean) {
    const ctx: StaffCtx = {
      me: makeStaffMeView({
        userId: ME,
        name: 'Ploy',
        roles: [],
        branchId: 'MAIN',
        branchName: 'XC-B',
        permissions: ['ShiftOperate'],
      }),
      floor: floor(open),
      refresh: vi.fn(async () => {}),
    };
    dispose = render(() => <ShiftCard ctx={ctx} />, host);
    await settle();
  }

  const click = async (sel: string) => {
    const el = host.querySelector<HTMLElement>(sel);
    expect(el, sel).not.toBeNull();
    el!.click();
    await settle();
  };
  const cancel = () => click('.xc-shift-cancel-btn');

  it('S1 + S2: opening the count form yields; cancelling it releases', async () => {
    await mount(false);
    expect(queueCardYielding(), 'nothing open yet').toBe(false);
    await click('[data-testid="shift-open-count"]');
    expect(host.querySelector('[data-testid="shift-count-form"]')).not.toBeNull();
    expect(queueCardYielding()).toBe(true);
    await cancel();
    expect(host.querySelector('[data-testid="shift-count-form"]')).toBeNull();
    expect(queueCardYielding()).toBe(false);
  });

  it('S3: the hand-over form yields while open and releases on cancel', async () => {
    await mount(true);
    await click('[data-testid="shift-handover"]');
    expect(queueCardYielding()).toBe(true);
    await cancel();
    expect(queueCardYielding()).toBe(false);
  });

  it('S4: unmounting the card with a form open releases the yield', async () => {
    await mount(true);
    await click('[data-testid="shift-close-count"]');
    expect(queueCardYielding()).toBe(true);
    dispose?.();
    dispose = undefined;
    expect(queueCardYielding()).toBe(false);
  });
});
