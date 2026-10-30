// #5639 (DESK-SHIFT T2, docs/design/desk-shift-open.md §2.1, §3 T2 rows N1–N6): the count form and waiting line
// /staff and /desk share. Written before the implementation by the ticket author claude/studio-claude-15.
// Do not edit, delete or weaken; add tests in a file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeCashCountRow,
  makeDenominationRow,
  makeMyShift,
  makeMyShiftBalance,
  makeStaffFloorView,
  makeStaffMeView,
} from '../../../shared/testing/fixtures';
import type { StaffCtx } from '../cards';
import { CountForm, CountWaitingLine } from './CountForm';
import { th } from './i18n';

const BRANCH = 'MAIN';
const ME = 'u-teller';
const SHIFT = 'shift-1';
const DENOMS = [
  makeDenominationRow({ id: 11, currency: 'THB', faceValue: '1000.0000', groupCode: 'NOTE' }),
  makeDenominationRow({ id: 12, currency: 'THB', faceValue: '100.0000', groupCode: 'NOTE' }),
];

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;
let views: string[] = [];
let posts: Array<{ path: string; body: Record<string, unknown> }> = [];
let postAnswer: () => Response | Promise<Response> = () => json(200, { id: 'cc-1' });
const shiftAnswer = () =>
  json(200, {
    version: 1,
    changed: true,
    data: makeMyShift({ id: SHIFT, balances: [makeMyShiftBalance({ code: 'THB', opening: '0', expected: '5000.00' })] }),
  });
let myShiftAnswer: () => Response | Promise<Response> = shiftAnswer;

function stub() {
  views = [];
  posts = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const path = url.split('?')[0]!;
      const method = (init?.method ?? 'GET').toUpperCase();
      if (method === 'GET') {
        const view = path.startsWith('/api/v3/views/') ? path.slice('/api/v3/views/'.length) : '';
        views.push(view);
        if (view === 'denominations') return json(200, { version: 1, changed: true, data: { rows: DENOMS } });
        if (view === 'my_shift') return myShiftAnswer();
        return json(404, { error: { code: 'notFound', message: 'x' } });
      }
      posts.push({ path, body: JSON.parse(String(init?.body ?? '{}')) });
      return postAnswer();
    }),
  );
}

function mount(kind: 'OPEN' | 'CLOSE', withDenoms = false) {
  const onSaved = vi.fn(async () => {});
  const onCancel = vi.fn();
  const onError = vi.fn();
  const [busy, setBusy] = createSignal(false);
  const [denoms] = createSignal(DENOMS);
  const ctx: StaffCtx = {
    me: makeStaffMeView({ userId: ME, name: 'Somchai', roles: [], branchId: BRANCH, branchName: 'XC-B', permissions: ['ShiftOperate'] }),
    floor: makeStaffFloorView({
      phase: 'OPEN',
      hours: null,
      myClockIn: '2026-10-11T01:00:00Z',
      myShift: kind === 'CLOSE' ? { id: SHIFT, openedAt: '2026-10-11T01:05:00Z' } : null,
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
        kind={kind}
        denominations={withDenoms ? denoms : undefined}
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

describe('#5639 CountForm', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
    postAnswer = () => json(200, { id: 'cc-1' });
    myShiftAnswer = shiftAnswer;
    stub();
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
  });

  it('N1 OPEN without a list: reads denominations itself and shows the THB inputs, no expected', async () => {
    mount('OPEN');
    await settle();
    expect(views).toContain('denominations');
    expect(views).not.toContain('my_shift');
    expect(input(11)).not.toBeNull();
    expect(input(12)).not.toBeNull();
    expect(host.querySelector('[data-testid="currency-expected"]')).toBeNull();
  });

  it('N1b with the host list: no denominations read', async () => {
    mount('OPEN', true);
    await settle();
    expect(views).not.toContain('denominations');
    expect(input(11)).not.toBeNull();
  });

  it('N2 CLOSE reads my_shift and shows the expected THB', async () => {
    mount('CLOSE', true);
    await settle();
    expect(views).toContain('my_shift');
    const exp = host.querySelector('[data-testid="currency-expected"]');
    expect(exp?.getAttribute('data-expected')).toBe('5000.00');
  });

  it('N2b CLOSE before my_shift has answered: save disabled and a submit sends nothing (#5631)', async () => {
    let release!: (r: Response) => void;
    myShiftAnswer = () => new Promise<Response>((r) => (release = r));
    mount('CLOSE', true);
    await settle();
    const save = () => host.querySelector<HTMLButtonElement>('[data-testid="shift-count-save"]')!;
    expect(save().disabled).toBe(true);
    type(11, '5');
    submit();
    await settle();
    expect(posts).toHaveLength(0);
    release(shiftAnswer());
    await settle();
    expect(save().disabled).toBe(false);
  });

  it('N2b my_shift fails: the form is usable again (THB only), as ShiftCard row 9 pins today', async () => {
    myShiftAnswer = () => json(500, { error: { code: 'internal', message: 'boom' } });
    mount('CLOSE', true);
    await settle();
    expect(host.querySelector<HTMLButtonElement>('[data-testid="shift-count-save"]')!.disabled).toBe(false);
  });

  it('N3 OPEN submit posts the count for my branch with no shift, then onSaved', async () => {
    const m = mount('OPEN', true);
    await settle();
    type(11, '3');
    submit();
    await settle();
    expect(posts).toHaveLength(1);
    expect(posts[0]!.path).toBe('/api/cash-counts');
    expect(posts[0]!.body).toMatchObject({
      branch: BRANCH,
      kind: 'OPEN',
      shiftId: null,
      lines: [{ currency: 'THB', denominationId: 11, qty: '3' }],
      note: null,
    });
    expect(m.onSaved).toHaveBeenCalledTimes(1);
    expect(m.busy()).toBe(false);
  });

  it('N3b CLOSE submit carries my shift id', async () => {
    mount('CLOSE', true);
    await settle();
    type(12, '2');
    submit();
    await settle();
    expect(posts[0]!.body).toMatchObject({ kind: 'CLOSE', shiftId: SHIFT });
  });

  it('N3c a bad quantity is worded on its line and nothing is sent', async () => {
    mount('OPEN', true);
    await settle();
    type(11, '-1');
    submit();
    await settle();
    expect(posts).toHaveLength(0);
    expect(host.querySelector('[data-field-error="lines.11.qty"]')?.textContent).toBe(th.qtyWholeMin0);
  });

  it('N3d a refusal on lines[0].qty shows on that line; any other refusal goes to onError; no onSaved', async () => {
    postAnswer = () => json(400, { error: { code: 'validation', message: 'bad', field: 'lines[0].qty' } });
    const m = mount('OPEN', true);
    await settle();
    type(12, '4');
    submit();
    await settle();
    expect(host.querySelector('[data-field-error="lines.12.qty"]')).not.toBeNull();
    expect(m.onSaved).not.toHaveBeenCalled();

    postAnswer = () => json(409, { error: { code: 'shiftAlreadyOpen', message: 'x' } });
    submit();
    await settle();
    expect(m.onError).toHaveBeenCalled();
    expect(m.onSaved).not.toHaveBeenCalled();
  });

  it('N4 a second submit while one is in flight sends nothing', async () => {
    let release!: (r: Response) => void;
    postAnswer = () => new Promise<Response>((r) => (release = r));
    const m = mount('OPEN', true);
    await settle();
    type(11, '1');
    submit();
    await settle();
    expect(m.busy()).toBe(true);
    submit();
    await settle();
    expect(posts).toHaveLength(1);
    release(json(200, { id: 'cc-1' }));
    await settle();
    expect(m.busy()).toBe(false);
  });

  it('N5 cancel calls onCancel and sends nothing', async () => {
    const m = mount('OPEN', true);
    await settle();
    const cancel = [...host.querySelectorAll<HTMLButtonElement>('button')].find(
      (b) => b.textContent?.trim() === th.cancel,
    );
    cancel!.click();
    await settle();
    expect(m.onCancel).toHaveBeenCalledTimes(1);
    expect(posts).toHaveLength(0);
  });
});

describe('#5639 CountWaitingLine', () => {
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  it('N6 OPEN and CLOSE lines read their text and the count time', () => {
    const row = makeCashCountRow({ id: 'c1', kind: 'OPEN', countedBy: ME, countedAt: '2026-10-11T04:30:00Z' });
    dispose = render(
      () => (
        <div>
          <CountWaitingLine kind="OPEN" row={row} />
          <CountWaitingLine kind="CLOSE" row={{ ...row, kind: 'CLOSE' }} />
        </div>
      ),
      host,
    );
    const open = host.querySelector('[data-testid="shift-open-waiting"]');
    const close = host.querySelector('[data-testid="shift-close-waiting"]');
    expect(open?.getAttribute('role')).toBe('status');
    expect(open?.textContent).toBe(`${th.openWaiting} (11/10 11:30)`);
    expect(close?.textContent).toBe(`${th.closeWaiting} (11/10 11:30)`);
  });
});
