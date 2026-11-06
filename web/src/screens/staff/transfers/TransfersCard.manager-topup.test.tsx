import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { JSX } from 'solid-js';
import {
  makeDenominationRow,
  makeOpenShiftDto,
  makeStaffFloorView,
  makeStaffMeView,
  type OpenShiftDto,
} from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { th, en } from './i18n';
import type { StaffCard, StaffCtx } from '../cards';

const cardMods = import.meta.glob<{ card: StaffCard }>('./card.ts');

async function loadCard(): Promise<StaffCard> {
  const l = cardMods['./card.ts'];
  expect(l).toBeDefined();
  return (await l!()).card;
}

const BRANCH = 'MAIN';
const ME = '00000000-0000-4000-8000-0000000000b1';

const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 21, currency: 'USD', faceValue: '50.0000', groupCode: 'NOTE' }),
];

type Sent = {
  url: string;
  method: string;
  key: string | null;
  body: string | undefined;
};

let sent: Sent[];
let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function stub() {
  const all: Record<string, unknown> = {
    cash_transfers: [],
    denominations: { rows: DENOMS },
    transfer_branches: { rows: [] },
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const name = path.startsWith('/api/v3/views/')
          ? path.slice('/api/v3/views/'.length)
          : '';
        if (name in all) {
          return json(200, { version: 1, changed: true, data: all[name] });
        }
        return json(404, { error: { code: 'notFound', message: 'not found' } });
      }
      const headers = new Headers(init?.headers);
      sent.push({
        url: path,
        method,
        key: headers.get('idempotency-key'),
        body: init?.body as string | undefined,
      });
      if (path === '/api/cash-transfers') {
        return json(200, { id: 'created-topup-tx', status: 'DRAFT' });
      }
      return json(200, { id: path.split('/')[3], status: 'DRAFT' });
    }),
  );
}

async function settle() {
  for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(opts: {
  myShiftId: string | null;
  openShifts: OpenShiftDto[];
  permissions?: string[];
}) {
  const card = await loadCard();
  const ctx: StaffCtx = {
    me: makeStaffMeView({
      userId: ME,
      name: 'Somchai',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: opts.permissions ?? ['CashManage'],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-11-05T01:00:00Z',
      myShift: opts.myShiftId
        ? { id: opts.myShiftId, openedAt: '2026-11-05T01:00:00Z' }
        : null,
      cutoff: null,
      target: null,
      safe: null,
      openShifts: opts.openShifts,
    }),
    refresh: vi.fn(async () => {}),
  };
  const C = card.Component as (p: { ctx: StaffCtx }) => JSX.Element;
  dispose = render(() => <C ctx={ctx} />, host);
  await settle();
}

function q<T extends Element = HTMLElement>(sel: string): T | null {
  return host.querySelector<T>(sel);
}

function must<T extends Element = HTMLElement>(sel: string): T {
  const el = q<T>(sel);
  expect(el, sel).not.toBeNull();
  return el!;
}

function tid(id: string): string {
  return `[data-testid="${id}"]`;
}

async function click(el: Element) {
  (el as HTMLElement).click();
  await settle();
}

function typeInto(
  input: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement,
  value: string,
) {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

beforeEach(() => {
  sent = [];
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  setLang('th');
});

describe('F37a (#6344): branch manager tops up a teller drawer from /staff', () => {
  it('1. manager, no own shift, openShifts = [T1 shift S1] -> SAFE_TO_DRAWER enabled, select lists "T1 · <time>", body toShiftId = S1 after choosing it', async () => {
    stub();
    await mount({
      myShiftId: null,
      openShifts: [
        makeOpenShiftDto({
          shiftId: 'S1',
          tellerId: 'u-t1',
          tellerName: 'T1',
          openedAt: '2026-11-05T01:05:00Z',
        }),
      ],
    });

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const s2dOpt = [...kindSelect.options].find((o) => o.value === 'SAFE_TO_DRAWER');
    expect(s2dOpt).toBeDefined();
    expect(s2dOpt!.disabled).toBe(false);

    const d2sOpt = [...kindSelect.options].find((o) => o.value === 'DRAWER_TO_SAFE');
    expect(d2sOpt).toBeDefined();
    expect(d2sOpt!.disabled).toBe(true);

    typeInto(kindSelect, 'SAFE_TO_DRAWER');
    await settle();

    const shiftSelect = must<HTMLSelectElement>(tid('transfer-to-shift'));
    expect(shiftSelect.parentElement?.textContent).toContain(th.pickTellerShift);
    const shiftOpt = [...shiftSelect.options].find((o) => o.value === 'S1');
    expect(shiftOpt).toBeDefined();
    expect(shiftOpt!.textContent).toBe('T1 · 08:05');

    typeInto(shiftSelect, 'S1');
    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'top up T1 drawer');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="21"]'), '12');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      'POST /api/cash-transfers',
      'POST /api/cash-transfers/created-topup-tx/submit',
    ]);
    const body = JSON.parse(sent[0]!.body!) as Record<string, unknown>;
    expect(body['kind']).toBe('SAFE_TO_DRAWER');
    expect(body['toShiftId']).toBe('S1');
    expect(body['fromShiftId']).toBeNull();
  });

  it('2. manager, openShifts = [] -> SAFE_TO_DRAWER disabled with noOpenShift (unchanged)', async () => {
    stub();
    await mount({
      myShiftId: null,
      openShifts: [],
    });

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const s2dOpt = [...kindSelect.options].find((o) => o.value === 'SAFE_TO_DRAWER');
    expect(s2dOpt).toBeDefined();
    expect(s2dOpt!.disabled).toBe(true);
    expect(s2dOpt!.textContent).toContain(th.noOpenShift);
    expect(q(tid('transfer-to-shift'))).toBeNull();
  });

  it('3. manager, nothing chosen, submit -> refused with pickTellerShift, no request', async () => {
    stub();
    await mount({
      myShiftId: null,
      openShifts: [
        makeOpenShiftDto({
          shiftId: 'S1',
          tellerId: 'u-t1',
          tellerName: 'T1',
          openedAt: '2026-11-05T01:05:00Z',
        }),
      ],
    });

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'SAFE_TO_DRAWER');
    await settle();

    expect(must<HTMLSelectElement>(tid('transfer-to-shift')).value).toBe('');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="21"]'), '12');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent).toEqual([]);
    expect(must(tid('transfer-error')).textContent).toBe(th.pickTellerShift);
    expect(must('[data-field-error="toShift"]').textContent).toBe(th.pickTellerShift);

    // English refusal check
    dispose?.();
    host.innerHTML = '';
    setLang('en');
    await mount({
      myShiftId: null,
      openShifts: [
        makeOpenShiftDto({
          shiftId: 'S1',
          tellerId: 'u-t1',
          tellerName: 'T1',
          openedAt: '2026-11-05T01:05:00Z',
        }),
      ],
    });

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());
    typeInto(must<HTMLSelectElement>(tid('transfer-kind')), 'SAFE_TO_DRAWER');
    await settle();
    typeInto(must<HTMLInputElement>('input[data-denomination-id="21"]'), '12');
    await settle();
    await click(must(tid('transfer-save')));

    expect(sent).toEqual([]);
    expect(must(tid('transfer-error')).textContent).toBe(en.pickTellerShift);
  });

  it('4. teller with own shift M -> unchanged, no select, body toShiftId = M', async () => {
    stub();
    await mount({
      myShiftId: 'M',
      openShifts: [],
      permissions: ['CashManage', 'ShiftOperate'],
    });

    await click(must(tid('transfer-new')));
    await vi.waitFor(() => expect(q(tid('transfer-form'))).not.toBeNull());

    const kindSelect = must<HTMLSelectElement>(tid('transfer-kind'));
    const s2dOpt = [...kindSelect.options].find((o) => o.value === 'SAFE_TO_DRAWER');
    expect(s2dOpt).toBeDefined();
    expect(s2dOpt!.disabled).toBe(false);

    typeInto(kindSelect, 'SAFE_TO_DRAWER');
    await settle();

    expect(q(tid('transfer-to-shift'))).toBeNull();

    typeInto(must<HTMLInputElement>(tid('transfer-note')), 'own shift topup');
    typeInto(must<HTMLInputElement>('input[data-denomination-id="11"]'), '5');
    await settle();

    await click(must(tid('transfer-save')));

    expect(sent.map((s) => `${s.method} ${s.url}`)).toEqual([
      'POST /api/cash-transfers',
      'POST /api/cash-transfers/created-topup-tx/submit',
    ]);
    const body = JSON.parse(sent[0]!.body!) as Record<string, unknown>;
    expect(body['kind']).toBe('SAFE_TO_DRAWER');
    expect(body['toShiftId']).toBe('M');
  });
});
