// Rate feed F3w: the admin screen for banknote groups and their rate-board mappings.
// Written before the implementation by the ticket author (claude/c954feda). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/rate-feed-connector.md` §10 (view `admin_denomination_groups`, the screen),
// §3.2 and owner's ruling R-3 on #986 ("ใช่ เจ้าของร้าน/แอดมินใน v3": the owner creates groups and maps
// them to the board's rows). Wire shapes: the commands of F3 at
// `api/src/presentation/http/v3/commands/denomination_groups.rs` —
// POST /api/denomination-groups `{currency, group}`, POST /api/denomination-groups/map
// `{currency, group, provider, instrumentKey}`, POST /api/denomination-groups/remove `{currency, group}`
// — each also carrying `branch` (the command endpoint reads it, `command.rs:197`).
//
// Behaviour this file locks:
// | case                                         | required result                                          |
// |----------------------------------------------|----------------------------------------------------------|
// | the menu record                              | id `denomination-groups`, roles `['OWNER']`, not dated, once |
// | the screen is shown                          | reads view `admin_denomination_groups` with the branch;  |
// |                                              | a currency `<select>` lists the view's currencies        |
// | a currency is picked                         | one `[data-group="<G>"]` per group, showing its board key |
// | add a valid group code                       | one POST /api/denomination-groups `{currency, group, branch}`, |
// |                                              | its own key; then re-reads                               |
// | add an invalid code (lower case, blank, -)   | nothing sent; shows `groupInvalid`                       |
// | save a board key in a group's row            | one POST …/map `{currency, group, provider: 'RATE_BOARD', instrumentKey, branch}` |
// | save an empty or blank key                   | nothing sent; shows `keyRequired`                        |
// | a removable group                            | has `remove` → POST …/remove `{currency, group, branch}`  |
// | `ALL`, or a group the view marks not removable | no `remove` button                                    |
// | a write refused by the server                | shows the server's `message`                             |
//
// DOM hooks and names this test assumes (the implementer follows them):
// - `menu.ts` exports `menu: AdminMenu` (id = folder name), texts in `./i18n.ts` (`th`, `en`).
// - The component builds its own `new ApiClient()`; the fake API is the global `fetch`.
// - A `<select>` labelled by the table's `currency`; each group is `[data-group="<GROUP>"]`, whose
//   text holds the group code and its board key (if any), and which holds an input labelled
//   `boardKey`, a button `save`, and — only when removable — a button `remove`.
// - The add form: an input labelled `newGroup` and a button `add`.
// - Keys of `./i18n.ts` this test reads: `label`, `currency`, `newGroup`, `add`, `groupInvalid`,
//   `boardKey`, `save`, `keyRequired`, `remove`.
//
// Amended for OR-7.7 (#1030) by the test-writer claude/5d45b017: the menu record's `roles: ['OWNER']` pin →
// `needs: 'RatesDraft'`, the permission its three commands already require (`INHERITS (Rpc, "rateDraft")`,
// `policy.rs`; lane controller's ruling on #1030 issuecomment-6105567502).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { menus, type AdminMenu, type AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { PAGE_WIDE, QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
interface TextsModule {
  th: Record<string, string>;
  en: Record<string, string>;
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const textFiles = import.meta.glob<TextsModule>('./i18n.ts', { eager: true });
const sources = import.meta.glob<string>(['./*.{ts,tsx}', '!./*.test.{ts,tsx}'], {
  query: '?raw',
  import: 'default',
  eager: true,
});

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/denomination-groups/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/denomination-groups/i18n.ts exists').toBeDefined();
  return m!;
}

const KEYS = ['label', 'currency', 'newGroup', 'add', 'groupInvalid', 'boardKey', 'save', 'keyRequired', 'remove'] as const;
type K = (typeof KEYS)[number];

function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const value = texts()[lang][key];
  expect(value, `admin.denomination-groups i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const VIEW = '/api/v3/views/admin_denomination_groups';
const CREATE = '/api/denomination-groups';
const MAP = '/api/denomination-groups/map';
const REMOVE = '/api/denomination-groups/remove';
const THAI = /[฀-๿]/;

// ── the fake API ─────────────────────────────────────────────────────────────

interface Group {
  group: string;
  instrumentKey: string | null;
  removable: boolean;
}
interface Currency {
  currency: string;
  groups: Group[];
}
interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}
type Reply = { status: number; body: unknown };

let sent: Sent[];
let data: Currency[];
let version: number;
let refusal: { status: number; code: string; message: string } | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function groupsOf(currency: string): Group[] {
  const c = data.find((x) => x.currency === currency);
  expect(c, `currency ${currency} in the fake`).toBeDefined();
  return c!.groups;
}

async function answer(s: Sent): Promise<Reply> {
  const { path, params: p, method, body } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: { kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' },
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };
  if (path === VIEW && method === 'GET') {
    const since = p.get('since');
    if (since !== null && Number(since) === version) return { status: 200, body: { version, changed: false } };
    return {
      status: 200,
      body: { version, changed: true, data: { provider: 'RATE_BOARD', currencies: structuredClone(data) } },
    };
  }
  if (method === 'POST' && [CREATE, MAP, REMOVE].includes(path)) {
    if (refusal) {
      return { status: refusal.status, body: { error: { code: refusal.code, message: refusal.message } } };
    }
    const currency = String(body?.currency);
    const group = String(body?.group);
    const groups = groupsOf(currency);
    if (path === CREATE) {
      if (!groups.some((g) => g.group === group)) groups.push({ group, instrumentKey: null, removable: true });
    } else if (path === MAP) {
      const g = groups.find((x) => x.group === group)!;
      g.instrumentKey = String(body?.instrumentKey);
    } else {
      const i = groups.findIndex((x) => x.group === group);
      groups.splice(i, 1);
    }
    version += 1;
    return { status: 200, body: { currency, group } };
  }
  return { status: 500, body: { error: { code: 'serverError', message: `unexpected ${method} ${path}` } } };
}

function headerOf(init: RequestInit | undefined, name: string): string | null {
  const h = init?.headers;
  if (!h) return null;
  if (h instanceof Headers) return h.get(name);
  if (Array.isArray(h)) return h.find(([k]) => k.toLowerCase() === name)?.[1] ?? null;
  const rec = h as Record<string, string>;
  const k = Object.keys(rec).find((x) => x.toLowerCase() === name);
  return k === undefined ? null : rec[k]!;
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  const s: Sent = { path: url.pathname, params: url.searchParams, method, key: headerOf(init, 'idempotency-key'), body };
  sent.push(s);
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

function reads(): Sent[] {
  return sent.filter((s) => s.path === VIEW && s.method === 'GET');
}
function writes(): Sent[] {
  return sent.filter((s) => s.method !== 'GET' && !PAGE_WIDE.includes(s.path));
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

// ── mounting ─────────────────────────────────────────────────────────────────

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

function mount(init: { branch?: string } = {}): HTMLElement {
  const menu = theMenu();
  const [branch] = createSignal(init.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-11', to: '2026-10-11' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () =>
      (
        <>
          <Component branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    el,
  );
  return el;
}

beforeEach(() => {
  sent = [];
  version = 1;
  data = [
    { currency: 'EUR', groups: [{ group: 'ALL', instrumentKey: null, removable: false }] },
    {
      currency: 'USD',
      groups: [
        { group: 'ALL', instrumentKey: 'USD', removable: false },
        { group: 'LARGE', instrumentKey: 'USD1', removable: true },
        { group: 'PRICED', instrumentKey: 'USD2', removable: false },
      ],
    },
  ];
  refusal = null;
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#denomination-groups';
  setLang('th');
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
  location.hash = '';
  setLang('th');
  await settle();
});

// ── helpers on the DOM ───────────────────────────────────────────────────────

function text(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function one(el: ParentNode, selector: string): HTMLElement {
  const found = el.querySelectorAll<HTMLElement>(selector);
  expect(found.length, `one ${selector}`).toBe(1);
  return found[0]!;
}

function buttons(el: ParentNode, name: string): HTMLButtonElement[] {
  return [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
}

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = buttons(el, name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  return found[0]!;
}

type Control = HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement;

function field(el: ParentNode, label: string): Control {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as Control | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function type(input: Control, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function pick(el: HTMLElement, currency: string): Promise<void> {
  type(field(el, say('currency')), currency);
  await settle();
}

async function shown(): Promise<HTMLElement> {
  const el = mount();
  await settle();
  await pick(el, 'USD');
  return el;
}

// ── the menu record ──────────────────────────────────────────────────────────

describe('F3w: the denomination-groups menu record (design §10.3)', () => {
  it('needs RatesDraft, not dated, reached through the shell glob once', () => {
    const menu = theMenu();
    expect(menu.id).toBe('denomination-groups');
    expect(menu.needs, 'its commands INHERITS (Rpc, "rateDraft") → RatesDraft (policy.rs)').toBe('RatesDraft');
    expect(menu.dated).toBe(false);
    expect(typeof menu.Component).toBe('function');
    const listed = menus.filter((m) => m.id === 'denomination-groups');
    expect(listed.length).toBe(1);
    expect(listed[0]).toBe(menu);
  });

  it('its table has every key in th and en, the en table has no Thai, and the label follows setLang', () => {
    const t = texts();
    for (const k of KEYS) {
      expect(t.th[k], `th.${k}`).toBeTruthy();
      expect(t.en[k], `en.${k}`).toBeTruthy();
    }
    expect(Object.keys(t.en).sort()).toEqual(Object.keys(t.th).sort());
    for (const [k, v] of Object.entries(t.en)) expect(THAI.test(v), `en.${k} has Thai`).toBe(false);
    const menu = theMenu();
    setLang('en');
    expect(menu.label()).toBe(say('label', 'en'));
    setLang('th');
    expect(menu.label()).toBe(say('label', 'th'));
  });

  it('goes through ApiClient, never a bare fetch', () => {
    const own = Object.entries(sources);
    expect(own.length, 'the folder has source files').toBeGreaterThan(0);
    for (const [file, src] of own) expect(/\bfetch\s*\(/.test(src), `${file} calls fetch directly`).toBe(false);
  });
});

// ── reading ──────────────────────────────────────────────────────────────────

describe('F3w: showing the groups (view admin_denomination_groups)', () => {
  it('reads the view for the picked branch and offers its currencies', async () => {
    const el = mount({ branch: 'B9' });
    await settle();
    expect(reads().length, 'the view is read on show').toBeGreaterThan(0);
    expect(reads()[0]!.params.get('branch')).toBe('B9');
    const select = field(el, say('currency')) as HTMLSelectElement;
    expect(select.tagName).toBe('SELECT');
    const options = [...select.options].map((o) => o.value).filter((v) => v !== '');
    expect(options).toEqual(['EUR', 'USD']);
  });

  it('a picked currency shows one row per group with its board key', async () => {
    const el = await shown();
    expect(el.querySelectorAll('[data-group]').length).toBe(3);
    expect(text(one(el, '[data-group="ALL"]'))).toContain('USD');
    const large = one(el, '[data-group="LARGE"]');
    expect(text(large)).toContain('LARGE');
    expect(text(large)).toContain('USD1');
  });

  it('only a removable group offers remove', async () => {
    const el = await shown();
    expect(buttons(one(el, '[data-group="ALL"]'), say('remove')).length, 'ALL').toBe(0);
    expect(buttons(one(el, '[data-group="PRICED"]'), say('remove')).length, 'not removable').toBe(0);
    expect(buttons(one(el, '[data-group="LARGE"]'), say('remove')).length, 'removable').toBe(1);
  });
});

// ── writing ──────────────────────────────────────────────────────────────────

describe('F3w: adding, mapping and removing (the F3 commands)', () => {
  it('adds a group with one POST under its own key, then re-reads', async () => {
    const el = await shown();
    const before = reads().length;
    type(field(el, say('newGroup')), 'SMALL');
    button(el, say('add')).click();
    await settle();
    expect(writes().length).toBe(1);
    const w = writes()[0]!;
    expect(w.path).toBe(CREATE);
    expect(w.method).toBe('POST');
    expect(w.body).toEqual({ currency: 'USD', group: 'SMALL', branch: 'MAIN' });
    expect(w.key, 'Idempotency-Key').toBeTruthy();
    expect(reads().length, 're-read after the write').toBeGreaterThan(before);
    expect(el.querySelectorAll('[data-group="SMALL"]').length).toBe(1);
  });

  it('an invalid group code sends nothing and says so', async () => {
    const el = await shown();
    for (const bad of ['small', '', '   ', 'LAR-GE', 'A'.repeat(33)]) {
      type(field(el, say('newGroup')), bad);
      button(el, say('add')).click();
      await settle();
    }
    expect(writes().length).toBe(0);
    expect(text(el)).toContain(say('groupInvalid'));
  });

  it('saves a board key with one POST to /map naming the board provider', async () => {
    const el = await shown();
    const row = one(el, '[data-group="LARGE"]');
    type(field(row, say('boardKey')), 'USD9');
    button(row, say('save')).click();
    await settle();
    expect(writes().length).toBe(1);
    const w = writes()[0]!;
    expect(w.path).toBe(MAP);
    expect(w.body).toEqual({
      currency: 'USD',
      group: 'LARGE',
      provider: 'RATE_BOARD',
      instrumentKey: 'USD9',
      branch: 'MAIN',
    });
    expect(w.key).toBeTruthy();
    expect(text(one(el, '[data-group="LARGE"]'))).toContain('USD9');
  });

  it('an empty or blank board key sends nothing and says it is required', async () => {
    const el = await shown();
    for (const blank of ['', '   ']) {
      const row = one(el, '[data-group="LARGE"]');
      type(field(row, say('boardKey')), blank);
      button(row, say('save')).click();
      await settle();
    }
    expect(writes().length).toBe(0);
    expect(text(el)).toContain(say('keyRequired'));
  });

  it('removes a removable group with one POST to /remove', async () => {
    const el = await shown();
    button(one(el, '[data-group="LARGE"]'), say('remove')).click();
    await settle();
    expect(writes().length).toBe(1);
    const w = writes()[0]!;
    expect(w.path).toBe(REMOVE);
    expect(w.body).toEqual({ currency: 'USD', group: 'LARGE', branch: 'MAIN' });
    expect(w.key).toBeTruthy();
    expect(el.querySelectorAll('[data-group="LARGE"]').length).toBe(0);
  });

  it('two writes carry two different keys', async () => {
    const el = await shown();
    type(field(el, say('newGroup')), 'SMALL');
    button(el, say('add')).click();
    await settle();
    type(field(el, say('newGroup')), 'TINY');
    button(el, say('add')).click();
    await settle();
    expect(writes().length).toBe(2);
    expect(writes()[0]!.key).not.toBe(writes()[1]!.key);
  });

  it('a refused write shows the server message', async () => {
    refusal = { status: 400, code: 'validationFailed', message: 'refused: a current publication prices it' };
    const el = await shown();
    button(one(el, '[data-group="LARGE"]'), say('remove')).click();
    await settle();
    expect(text(document.body)).toContain('refused: a current publication prices it');
  });
});
