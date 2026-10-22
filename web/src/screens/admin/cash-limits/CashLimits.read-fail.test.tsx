// R3-1 (docs/design/web-silent-failures.md §7.2): a failed `cash_limits` read must never leave the
// form showing (and saving) a limit it did not read for the current branch. Up-front tests: rows
// (a), (b), (c) and (e) are red until `CashLimits.tsx` keeps a `known` state; row (d) is a guard.
// The worded refusal alert stays exactly as today and stays the only `role="alert"`; `unreadable` is
// plain banner text with a `type="button"` retry beside it, and it is Save's title (§7.2 at 01e980a6).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX, type Setter } from 'solid-js';
import { render } from 'solid-js/web';
import { makeBranchRef, makeCashLimitsData, makeMe, type CashLimitsData, type Me } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';
import { menu } from './menu';
import * as texts from './i18n';

const NO_ROW = makeCashLimitsData({ branch: 'MAIN', countDifferenceLimitThb: null, updatedBy: null, updatedAt: null });

const ROW_500 = makeCashLimitsData({
  branch: 'MAIN',
  countDifferenceLimitThb: '500.00',
  updatedBy: 'u-admin',
  updatedAt: '2026-10-12T01:00:00Z',
});

const ROW_B1_250 = makeCashLimitsData({
  branch: 'B1',
  countDifferenceLimitThb: '250.50',
  updatedBy: 'u-b1',
  updatedAt: '2026-10-12T02:00:00Z',
});

/** A read answer: data, a failure, or a held promise of data. */
type Read = CashLimitsData | { fail: number } | (() => Promise<CashLimitsData>);

interface Sent {
  method: string;
  path: string;
  query: URLSearchParams;
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

class Server {
  me: Me = makeMe({
    id: 'u-fin',
    roles: [],
    permissions: ['CashLimitsManage'],
    branch: 'MAIN',
    branches: [makeBranchRef({ id: 'MAIN', code: 'MAIN' }), makeBranchRef({ id: 'B1', code: 'B1' })],
  });
  readsByBranch: Record<string, Read> = { MAIN: NO_ROW };
  sent: Sent[] = [];

  fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const u = new URL(raw, 'http://xc.test');
    const method = (init?.method ?? 'GET').toUpperCase();
    this.sent.push({ method, path: u.pathname, query: u.searchParams });
    const view = (data: unknown) => json(200, { version: 1, changed: true, data });
    if (method === 'GET') {
      switch (u.pathname) {
        case '/api/v3/views/me':
          return view(this.me);
        case QUEUE_CARD_PATH:
          return json(200, queueCardUnchanged());
        case '/api/v3/views/cash_limits': {
          const b = u.searchParams.get('branch') ?? 'MAIN';
          const r = this.readsByBranch[b] ?? { ...NO_ROW, branch: b };
          if (typeof r === 'function') return view(await r());
          if ('fail' in r) return json(r.fail, { error: { code: 'serverError', message: 'boom' } });
          return view(r);
        }
        default:
          return view(null);
      }
    }
    if (method === 'PUT' && u.pathname === '/api/cash-limits') {
      return json(200, ROW_500);
    }
    return json(404, { error: { code: 'notFound', message: `unexpected ${method} ${u.pathname}` } });
  };

  writes(): Sent[] {
    return this.sent.filter((s) => s.method === 'PUT' && s.path === '/api/cash-limits');
  }

  reads(): Sent[] {
    return this.sent.filter((s) => s.method === 'GET' && s.path === '/api/v3/views/cash_limits');
  }
}

let server: Server;
let dispose: (() => void) | undefined;
let root: HTMLElement;
let setBranchSignal: Setter<string>;

async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(readsByBranch: Record<string, Read>): Promise<HTMLElement> {
  server.readsByBranch = readsByBranch;
  const [branch, setBranch] = createSignal('MAIN');
  setBranchSignal = setBranch;
  const [period] = createSignal({ from: '2026-10-12', to: '2026-10-12' });
  const [shown] = createSignal(true);
  const Component = (menu as AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element }).Component;
  root = document.createElement('div');
  document.body.appendChild(root);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    root,
  );
  await settle();
  return root.querySelector<HTMLElement>('[data-section="cash-limits"]')!;
}

beforeEach(() => {
  server = new Server();
  vi.stubGlobal('fetch', vi.fn(server.fetch));
  location.hash = '#cash-limits';
  setLang('th');
});

afterEach(async () => {
  await settle();
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
});

const text = (el: Element | null | undefined) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const limitInput = (s: ParentNode) => s.querySelector<HTMLInputElement>('input[name="countDifferenceLimitThb"]')!;
const saveButton = (s: ParentNode) => s.querySelector<HTMLButtonElement>('button[data-action="save"]')!;
const banner = (s: ParentNode) => s.querySelector<HTMLElement>('[data-current-limit]');
const alerts = (s: ParentNode) => Array.from(s.querySelectorAll('[role="alert"]'));
const retryButton = (s: ParentNode) =>
  Array.from(s.querySelectorAll<HTMLButtonElement>('button[type="button"]')).find(
    (b) => text(b) === texts.th.retry,
  ) ?? null;

function typeInput(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

/** Submits the form directly, as Enter in the input would, bypassing a disabled Save button. */
async function submitForm(s: HTMLElement): Promise<void> {
  s.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
  await settle();
}

describe('CashLimits read failure (R3-1)', () => {
  it('(a) a failed first read shows unreadable in the banner with a retry, keeps the worded alert, no "no limit", and Save is disabled and sends nothing', async () => {
    const s = await mount({ MAIN: { fail: 500 } });

    expect(text(banner(s)), 'the banner reads unreadable').toContain(texts.th.unreadable);
    expect(retryButton(s), 'a type="button" retry').not.toBeNull();
    expect(text(s)).not.toContain(texts.th.noLimit);
    expect(alerts(s).map(text), 'the worded refusal stays the only alert').toEqual([sharedTh['error.serverError']]);
    expect(saveButton(s).disabled).toBe(true);
    expect(saveButton(s).title).toBe(texts.th.unreadable);

    typeInput(limitInput(s), '100.00');
    await submitForm(s);
    expect(server.writes()).toHaveLength(0);
  });

  it('(b) after branch A loaded 500.00, a failed read for B shows no 500.00 anywhere, Save is disabled, and a submit sends no PUT', async () => {
    const s = await mount({ MAIN: ROW_500, B1: { fail: 500 } });
    expect(limitInput(s).value).toBe('500.00');

    setBranchSignal('B1');
    await settle();

    expect(text(s)).not.toContain('500.00');
    expect(limitInput(s).value).not.toBe('500.00');
    expect(text(banner(s))).toContain(texts.th.unreadable);
    expect(saveButton(s).disabled).toBe(true);
    await submitForm(s);
    expect(server.writes()).toHaveLength(0);
  });

  it('(c) after a failed first read, a successful retry removes unreadable and the alert, enables Save and shows the value', async () => {
    const s = await mount({ MAIN: { fail: 500 } });
    const retry = retryButton(s);
    expect(retry, 'a type="button" retry').not.toBeNull();

    server.readsByBranch = { MAIN: ROW_500 };
    retry!.click();
    await settle();

    expect(server.reads().map((r) => r.query.get('branch'))).toEqual(['MAIN', 'MAIN']);
    expect(text(s)).not.toContain(texts.th.unreadable);
    expect(alerts(s)).toHaveLength(0);
    expect(saveButton(s).disabled).toBe(false);
    expect(text(banner(s))).toContain('500.00');
    expect(limitInput(s).value).toBe('500.00');
  });

  it('(d) guard: a successful read answering no limit shows the "no limit" banner and Save is enabled', async () => {
    const s = await mount({ MAIN: NO_ROW });

    expect(text(banner(s))).toContain(texts.th.noLimit);
    expect(saveButton(s).disabled).toBe(false);
    expect(text(s)).not.toContain(texts.th.unreadable);
    expect(retryButton(s)).toBeNull();
  });

  it('(e) a branch switch clears the form before the new read answers', async () => {
    let answerB1!: (d: CashLimitsData) => void;
    const heldB1 = new Promise<CashLimitsData>((r) => {
      answerB1 = r;
    });
    const s = await mount({ MAIN: ROW_500, B1: () => heldB1 });
    expect(limitInput(s).value).toBe('500.00');

    setBranchSignal('B1');
    await settle();

    expect(text(s)).not.toContain('500.00');
    expect(limitInput(s).value).not.toBe('500.00');

    answerB1(ROW_B1_250);
    await settle();
    expect(limitInput(s).value).toBe('250.50');
  });
});
