// #6326 (F35): an empty cash count says to count an empty drawer as 0 (§8 rows 1–3).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeDenominationRow,
  makeOpenShiftChoice,
  makeOpenShiftChoices,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { CountForm } from './CountForm';
import { th } from './i18n';

const BRANCH = 'MAIN';
const ME = 'u-teller';
const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
];
const ONE_SHIFT = makeOpenShiftChoices({
  choices: [makeOpenShiftChoice({ templateId: 'tpl-1' })],
  preselect: 'tpl-1',
});

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let posts: Array<{ path: string; body: Record<string, unknown> }> = [];

function stub() {
  posts = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const view = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        if (view === 'open_shift_choices') return json(200, { version: 1, changed: true, data: ONE_SHIFT });
        return json(404, { error: { code: 'notFound', message: 'x' } });
      }
      posts.push({ path, body: JSON.parse(String(init?.body ?? '{}')) });
      return json(200, { id: 'cc-1' });
    }),
  );
}

function mount() {
  const onSaved = vi.fn(async () => {});
  const onCancel = vi.fn();
  const onError = vi.fn();
  const [busy, setBusy] = createSignal(false);
  const [denoms] = createSignal(DENOMS);
  const ctx: StaffCtx = {
    me: makeStaffMeView({
      userId: ME,
      name: 'Somchai',
      roles: [],
      branchId: BRANCH,
      branchName: 'XC-B',
      permissions: ['ShiftOperate'],
    }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-10-11T01:00:00Z',
      myShift: null,
      cutoff: null,
      target: null,
      safe: null,
    }),
    refresh: vi.fn(async () => {}),
  };
  dispose = render(
    () => (
      <CountForm
        ctx={ctx}
        kind="OPEN"
        denominations={denoms}
        busy={busy}
        setBusy={setBusy}
        onSaved={onSaved}
        onCancel={onCancel}
        onError={onError}
      />
    ),
    host,
  );
  return { onSaved, onCancel, onError, busy };
}

async function settle() {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

const input = (id: number) => host.querySelector<HTMLInputElement>(`input[data-denomination-id="${id}"]`);
function type(id: number, v: string) {
  const el = input(id)!;
  el.value = v;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}
const form = () => host.querySelector<HTMLFormElement>('[data-testid="shift-count-form"]');
const submit = () => form()!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));

describe('#6326 CountForm empty count', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('every box blank: no POST /api/cash-counts and countNeedsALine error shown', async () => {
    const m = mount();
    await settle();
    submit();
    await settle();
    expect(posts).toHaveLength(0);
    expect(m.onError).toHaveBeenCalledWith(th.countNeedsALine);
    expect(m.onSaved).not.toHaveBeenCalled();
  });

  it('one box 0, rest blank: sent with that one line (unchanged)', async () => {
    const m = mount();
    await settle();
    type(11, '0');
    submit();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/cash-counts');
    expect(posts[0]!.body).toMatchObject({
      lines: [{ currency: 'THB', denominationId: 11, qty: '0' }],
    });
    expect(m.onError).not.toHaveBeenCalledWith(th.countNeedsALine);
    expect(m.onSaved).toHaveBeenCalledTimes(1);
  });

  it('one box abc: the line error qtyWholeMin0 (unchanged), no POST or form error', async () => {
    const m = mount();
    await settle();
    type(11, 'abc');
    submit();
    await settle();
    expect(posts).toHaveLength(0);
    expect(host.querySelector('[data-field-error="lines.11.qty"]')?.textContent).toBe(th.qtyWholeMin0);
    expect(m.onError).not.toHaveBeenCalled();
    expect(m.onSaved).not.toHaveBeenCalled();
  });
});
