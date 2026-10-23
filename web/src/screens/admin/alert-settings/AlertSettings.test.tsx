// EX-5 (#1075): A01's alert thresholds, the form.
// Written before the implementation by the ticket author (claude/89bd61d9). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// Design: `docs/design/blueprint/executive-finance.md` §6.1 (the save, its concurrency), §8.2 (the
// two routes), §11.1 EX-5 ("the form loads the set, sends expectedVersion, shows a stale refusal and
// re-reads"); owner decisions E-3 (no threshold = no alert) and E-6 (no last-write-wins).
// The API side is pinned by `api/tests/branch_alert_settings_test.rs`.
//
// Signatures, props, DTO fields and DOM hooks this test assumes (the implementer follows them):
// - The menu is `./menu.ts`'s `menu.Component(props: AdminMenuProps)`, id `alert-settings` (the
//   folder name, as `collectMenus` requires), mounted as the shell mounts it, beside the page's one
//   `Dialog` and `Toast`. It builds its own `new ApiClient()`, so the fake API is the global `fetch`.
//   AppShell's page-wide reads (`PAGE_WIDE`, `shared/testing/page-wide.ts`) are
//   answered and never counted as the menu's own. Which roles see the menu is not pinned here: the
//   menu's field moves with OR-7.7 #1030, and the API refuses every other role.
// - On show it reads `GET /api/v3/views/branch_alert_settings?branch=<picked branch>`, which answers
//   `{ branch, staleDataMinutes: number | null, floors: [{ currency, minQuantity }], version:
//   number | null }` inside the v3 view envelope.
// - Save is `POST /api/v3/commands/branch_alert_settings_save` through `ApiClient.command`, with a
//   non-empty `idempotency-key` of its own and the body `{ branch, staleDataMinutes, floors:
//   [{ currency, minQuantity }], expectedVersion }`. `minQuantity` is the decimal string as typed;
//   an empty minutes field is `null` (E-3). `expectedVersion` is the version of the set on screen:
//   the view's, or the last save's answer.
// - A refused save shows `wordedError` (`error.staleVersion` or the server's `message`). A `staleVersion` refusal also reads the view again,
//   so the screen shows the newer set and the next save carries its version.
// - DOM hooks: the minutes input is found by the `<label>` whose text starts with the i18n
//   `staleDataMinutes`; a floor row is `[data-floor="<CODE>"]` and holds one `<input>` for its
//   `minQuantity`; the save button is the button whose whole text is the i18n `save`.
// - Adding and removing a floor (lane controller's ruling on #1075, review of PR 1324; design E-3 and
//   D-6: floors are rows the owner enters per currency). The currency-code input is found by the
//   `<label>` whose text starts with the i18n `addCurrency`, and the button whose whole text is the i18n
//   `addFloor` adds a row `[data-floor="<CODE>"]` with an empty `minQuantity`. A code already listed adds
//   no second row. Each row holds a button whose whole text is the i18n `removeFloor`, which drops that
//   row. The save sends the rows on screen, so removing every row sends `floors: []`. The screen
//   does not check a code against the register: the server refuses one it does not know
//   (`validationFailed`, field `floors`) and the screen shows that message.
// - Keys of `./i18n.ts` (th and en; the wording is the implementer's): `staleDataMinutes`, `save`,
//   `addCurrency`, `addFloor`, `removeFloor`.
//
// The menu and its table are read through eager globs, so on a branch without them each test fails
// on an assertion that the file exists, not on the import.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { meOf } from '../../../test-support/me';
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

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/alert-settings/menu.ts exists').toBeDefined();
  return m!.menu;
}

const KEYS = ['staleDataMinutes', 'save', 'addCurrency', 'addFloor', 'removeFloor'] as const;
type K = (typeof KEYS)[number];

/** The screen's own text for `key` in the current test's language. */
function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/alert-settings/i18n.ts exists').toBeDefined();
  const value = m![lang][key];
  expect(value, `admin.alert-settings i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const VIEW = '/api/v3/views/branch_alert_settings';
const SAVE = '/api/v3/commands/branch_alert_settings_save';

const B01 = 'XC-B01';
/** The codes the fake server's `currencies` register holds. */
const REGISTER = ['USD', 'JPY', 'EUR', 'GBP'];
const B09 = 'XC-B09';
const THAI = /[฀-๿]/;

// ── the fake server ──────────────────────────────────────────────────────────

interface Floor {
  currency: string;
  minQuantity: string;
}
interface Settings {
  branch: string;
  staleDataMinutes: number | null;
  floors: Floor[];
  version: number | null;
}
interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, any> | undefined;
}
interface Reply {
  status: number;
  body: unknown;
}

let sent: Sent[];
let stored: Record<string, Settings>;
/** Moves the stored set of `branch` as another editor would, after the screen has read it. */
let otherEditor: ((branch: string) => void) | null;

function empty(branch: string): Settings {
  return { branch, staleDataMinutes: null, floors: [], version: null };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function refusal(status: number, code: string, message: string, field?: string): Reply {
  return { status, body: { error: { code, message, ...(field ? { field } : {}) } } };
}

function answer(s: Sent): Reply {
  const { path, params, method, body } = s;
  if (path === '/api/v3/views/me') {
    return {
      status: 200,
      body: {
        version: 1,
        changed: true,
        data: meOf({ id: 'u-exec', role: 'EXECUTIVE', branch: B01, name: 'Exec', orgWide: true }),
      },
    };
  }
  if (path === QUEUE_CARD_PATH) return { status: 200, body: queueCardUnchanged() };
  if (path === VIEW && method === 'GET') {
    const branch = params.get('branch') ?? '';
    const data = structuredClone(stored[branch] ?? empty(branch));
    const v = 1000 + (data.version ?? 0);
    if (params.get('since') !== null && Number(params.get('since')) === v) {
      return { status: 200, body: { version: v, changed: false } };
    }
    return { status: 200, body: { version: v, changed: true, data } };
  }
  if (path === SAVE && method === 'POST') {
    const branch = String(body?.branch ?? '');
    if (otherEditor) {
      otherEditor(branch);
      otherEditor = null;
    }
    for (const f of (body?.floors ?? []) as Floor[]) {
      if (!REGISTER.includes(f.currency)) {
        return refusal(400, 'validationFailed', `Currency ${f.currency} is not in the register (test)`, 'floors');
      }
    }
    const cur = stored[branch] ?? empty(branch);
    if ((body?.expectedVersion ?? null) !== cur.version) {
      return refusal(409, 'staleVersion', 'Someone saved these thresholds first (test)');
    }
    const next: Settings = {
      branch,
      staleDataMinutes: (body?.staleDataMinutes ?? null) as number | null,
      floors: ((body?.floors ?? []) as Floor[]).map((f) => ({ currency: f.currency, minQuantity: f.minQuantity })),
      version: (cur.version ?? 0) + 1,
    };
    stored[branch] = next;
    return { status: 200, body: structuredClone(next) };
  }
  return refusal(500, 'serverError', `unexpected ${method} ${path}`);
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
  const reply = answer(s);
  return json(reply.status, reply.body);
}

function readsOf(branch: string): Sent[] {
  return sent.filter((s) => s.path === VIEW && s.method === 'GET' && s.params.get('branch') === branch);
}
function saves(): Sent[] {
  return sent.filter((s) => s.path === SAVE && s.method === 'POST');
}
function ownCalls(): Sent[] {
  return sent.filter((s) => !PAGE_WIDE.includes(s.path));
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

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(init: { branch?: string } = {}): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? B01);
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
  return { el, setBranch };
}

beforeEach(() => {
  sent = [];
  otherEditor = null;
  stored = {
    [B01]: {
      branch: B01,
      staleDataMinutes: 30,
      floors: [
        { currency: 'USD', minQuantity: '1000' },
        { currency: 'JPY', minQuantity: '50000' },
      ],
      version: 4,
    },
  };
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#alert-settings';
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

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)}`).toBe(1);
  expect(found[0]!.disabled, `the button ${JSON.stringify(name)} is enabled`).toBe(false);
  return found[0]!;
}

function field(el: ParentNode, label: string): HTMLInputElement {
  const found = [...el.querySelectorAll('label')].filter((l) => text(l).startsWith(label));
  expect(found.length, `one label starting ${JSON.stringify(label)}`).toBe(1);
  const control = (found[0] as HTMLLabelElement).control as HTMLInputElement | null;
  expect(control, `the control of ${label}`).not.toBeNull();
  return control!;
}

function floorInput(el: ParentNode, code: string): HTMLInputElement {
  const rows = el.querySelectorAll<HTMLElement>(`[data-floor="${code}"]`);
  expect(rows.length, `one floor row [data-floor="${code}"]`).toBe(1);
  const inputs = rows[0]!.querySelectorAll<HTMLInputElement>('input');
  expect(inputs.length, `one input in the ${code} floor row`).toBe(1);
  return inputs[0]!;
}

function floorCodes(el: ParentNode): string[] {
  return [...el.querySelectorAll<HTMLElement>('[data-floor]')].map((r) => r.dataset.floor ?? '').sort();
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function pageText(): string {
  return text(document.body);
}

async function save(el: ParentNode, lang: 'th' | 'en' = 'th'): Promise<void> {
  button(el, say('save', lang)).click();
  await settle();
}

/** The floors a save sent, keyed by currency. */
function sentFloors(s: Sent): Record<string, unknown> {
  const floors = (s.body?.floors ?? []) as Array<Record<string, unknown>>;
  expect(Array.isArray(floors), 'the save carries floors').toBe(true);
  const out: Record<string, unknown> = {};
  for (const f of floors) out[String(f.currency)] = f.minQuantity;
  return out;
}

/** The button whose whole text is `name` inside the floor row of `code`. */
function rowButton(el: ParentNode, code: string, name: string): HTMLButtonElement {
  const rows = el.querySelectorAll<HTMLElement>(`[data-floor="${code}"]`);
  expect(rows.length, `one floor row [data-floor="${code}"]`).toBe(1);
  const found = [...rows[0]!.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
  expect(found.length, `one button ${JSON.stringify(name)} in the ${code} row`).toBe(1);
  return found[0]!;
}

async function addFloor(el: ParentNode, code: string): Promise<void> {
  type(field(el, say('addCurrency')), code);
  button(el, say('addFloor')).click();
  await settle();
}

async function removeFloor(el: ParentNode, code: string): Promise<void> {
  rowButton(el, code, say('removeFloor')).click();
  await settle();
}

// ── tests ────────────────────────────────────────────────────────────────────

describe('EX-5 A01 thresholds form (#1075; design §6.1, §8.2, §11.1)', () => {
  it("is the 'alert-settings' menu", () => {
    expect(theMenu().id).toBe('alert-settings');
  });

  it('on show it reads branch_alert_settings for the picked branch, writes nothing, and shows the set', async () => {
    const m = mount();
    await settle();
    expect(readsOf(B01).length, 'the view is read for XC-B01').toBeGreaterThanOrEqual(1);
    expect(saves(), 'showing the form saves nothing').toEqual([]);
    for (const s of ownCalls()) {
      expect(s.path, `the form reads only its own view: ${s.method} ${s.path}`).toBe(VIEW);
    }
    expect(field(m.el, say('staleDataMinutes')).value).toBe('30');
    expect(floorCodes(m.el)).toEqual(['JPY', 'USD']);
    expect(floorInput(m.el, 'USD').value).toBe('1000');
    expect(floorInput(m.el, 'JPY').value).toBe('50000');
  });

  it('a save sends the whole set with expectedVersion = the version shown, an Idempotency-Key, and minQuantity as a string', async () => {
    const m = mount();
    await settle();
    type(field(m.el, say('staleDataMinutes')), '45');
    type(floorInput(m.el, 'USD'), '1200.5');
    await save(m.el);

    const s = saves();
    expect(s.length, 'one save').toBe(1);
    expect(s[0]!.key, 'an Idempotency-Key').toBeTruthy();
    expect(s[0]!.body?.branch).toBe(B01);
    expect(s[0]!.body?.staleDataMinutes).toBe(45);
    expect(s[0]!.body?.expectedVersion, 'the version the view answered').toBe(4);
    expect(sentFloors(s[0]!)).toEqual({ USD: '1200.5', JPY: '50000' });
    expect(stored[B01]!.version, 'the fake server took it').toBe(5);
  });

  it('a second save carries the version the first save answered, with a key of its own', async () => {
    const m = mount();
    await settle();
    type(field(m.el, say('staleDataMinutes')), '40');
    await save(m.el);
    type(field(m.el, say('staleDataMinutes')), '50');
    await save(m.el);

    const s = saves();
    expect(s.length, 'two saves').toBe(2);
    expect(s[0]!.body?.expectedVersion).toBe(4);
    expect(s[1]!.body?.expectedVersion, "the first save's answer, not the first read's").toBe(5);
    expect(s[1]!.body?.staleDataMinutes).toBe(50);
    expect(s[0]!.key).toBeTruthy();
    expect(s[1]!.key).toBeTruthy();
    expect(s[1]!.key, 'each save has a key of its own').not.toBe(s[0]!.key);
    expect(pageText()).not.toContain('Someone saved these thresholds first (test)');
  });

  it('a branch with no thresholds shows an empty set and its first save sends expectedVersion null', async () => {
    const m = mount({ branch: B09 });
    await settle();
    expect(readsOf(B09).length).toBeGreaterThanOrEqual(1);
    expect(field(m.el, say('staleDataMinutes')).value).toBe('');
    expect(floorCodes(m.el)).toEqual([]);
    type(field(m.el, say('staleDataMinutes')), '20');
    await save(m.el);

    const s = saves();
    expect(s.length).toBe(1);
    expect(s[0]!.body?.branch).toBe(B09);
    expect('expectedVersion' in (s[0]!.body ?? {}), 'expectedVersion is sent, as null').toBe(true);
    expect(s[0]!.body?.expectedVersion).toBeNull();
    expect(s[0]!.body?.staleDataMinutes).toBe(20);
    expect(stored[B09]!.version).toBe(1);
  });

  it('an empty minutes field is sent as null: no stale-data alert (E-3)', async () => {
    const m = mount();
    await settle();
    type(field(m.el, say('staleDataMinutes')), '');
    await save(m.el);

    const s = saves();
    expect(s.length).toBe(1);
    expect('staleDataMinutes' in (s[0]!.body ?? {}), 'staleDataMinutes is sent, as null').toBe(true);
    expect(s[0]!.body?.staleDataMinutes).toBeNull();
    expect(sentFloors(s[0]!)).toEqual({ USD: '1000', JPY: '50000' });
  });

  it('a staleVersion refusal is shown, the view is read again, and the next save carries the newer version', async () => {
    const m = mount();
    await settle();
    otherEditor = (branch) => {
      stored[branch] = {
        branch,
        staleDataMinutes: 15,
        floors: [{ currency: 'USD', minQuantity: '800' }],
        version: 5,
      };
    };
    const readsBefore = readsOf(B01).length;
    type(field(m.el, say('staleDataMinutes')), '45');
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(saves()[0]!.body?.expectedVersion).toBe(4);
    expect(pageText(), 'the refusal is shown, not swallowed').toContain('ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง');
    expect(readsOf(B01).length, 'the view is read again after staleVersion').toBeGreaterThan(readsBefore);
    expect(field(m.el, say('staleDataMinutes')).value, "the other editor's set is shown").toBe('15');
    expect(floorCodes(m.el)).toEqual(['USD']);
    expect(floorInput(m.el, 'USD').value).toBe('800');
    expect(stored[B01]!.staleDataMinutes, 'the refused save wrote nothing').toBe(15);

    await save(m.el);
    expect(saves().length).toBe(2);
    expect(saves()[1]!.body?.expectedVersion, 'the version read again').toBe(5);
    expect(stored[B01]!.version).toBe(6);
  });

  it('another refusal shows the server message', async () => {
    const m = mount();
    await settle();
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: string, init?: RequestInit) => {
        const url = new URL(input, 'http://localhost');
        if (url.pathname === SAVE) {
          sent.push({ path: SAVE, params: url.searchParams, method: 'POST', key: headerOf(init, 'idempotency-key'), body: undefined });
          return json(400, { error: { code: 'validationFailed', message: 'Minutes must be above zero (test)', field: 'staleDataMinutes' } });
        }
        return fakeFetch(input, init);
      }),
    );
    type(field(m.el, say('staleDataMinutes')), '10');
    await save(m.el);
    expect(saves().length).toBe(1);
    expect(pageText()).toContain('Minutes must be above zero (test)');
  });

  it('a branch change reads the other branch and saves go to it', async () => {
    const m = mount();
    await settle();
    m.setBranch(B09);
    await settle();
    expect(readsOf(B09).length).toBeGreaterThanOrEqual(1);
    expect(field(m.el, say('staleDataMinutes')).value).toBe('');
    expect(floorCodes(m.el)).toEqual([]);
    type(field(m.el, say('staleDataMinutes')), '25');
    await save(m.el);
    expect(saves()[0]!.body?.branch).toBe(B09);
    expect(saves()[0]!.body?.expectedVersion).toBeNull();
    expect(stored[B01]!.version, 'XC-B01 untouched').toBe(4);
  });
});

describe('EX-5 adding and removing a floor (#1075 ruling on PR 1324; design E-3, D-6)', () => {
  it('adds a floor for a currency the branch has none for, and the save sends it with the others', async () => {
    const m = mount();
    await settle();
    await addFloor(m.el, 'EUR');
    expect(floorCodes(m.el)).toEqual(['EUR', 'JPY', 'USD']);
    expect(floorInput(m.el, 'EUR').value, 'a new floor starts empty').toBe('');
    type(floorInput(m.el, 'EUR'), '200');
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(saves()[0]!.body?.expectedVersion).toBe(4);
    expect(sentFloors(saves()[0]!)).toEqual({ USD: '1000', JPY: '50000', EUR: '200' });
    expect(stored[B01]!.floors.map((f) => f.currency).sort()).toEqual(['EUR', 'JPY', 'USD']);
  });

  it('on a branch with no thresholds the first floor can be entered and saved (E-3)', async () => {
    const m = mount({ branch: B09 });
    await settle();
    expect(floorCodes(m.el)).toEqual([]);
    await addFloor(m.el, 'USD');
    type(floorInput(m.el, 'USD'), '500');
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(saves()[0]!.body?.branch).toBe(B09);
    expect(saves()[0]!.body?.expectedVersion).toBeNull();
    expect(sentFloors(saves()[0]!)).toEqual({ USD: '500' });
    expect(stored[B09]!.floors).toEqual([{ currency: 'USD', minQuantity: '500' }]);
  });

  it('a code already listed adds no second row', async () => {
    const m = mount();
    await settle();
    await addFloor(m.el, 'USD');
    expect(floorCodes(m.el)).toEqual(['JPY', 'USD']);
    expect(floorInput(m.el, 'USD').value, 'the existing floor is kept').toBe('1000');
  });

  it('removes a floor, and the save sends the set without it', async () => {
    const m = mount();
    await settle();
    await removeFloor(m.el, 'USD');
    expect(floorCodes(m.el)).toEqual(['JPY']);
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(sentFloors(saves()[0]!)).toEqual({ JPY: '50000' });
    expect(stored[B01]!.floors).toEqual([{ currency: 'JPY', minQuantity: '50000' }]);
  });

  it('removing every floor sends floors: [] (no low-stock alert, E-3)', async () => {
    const m = mount();
    await settle();
    await removeFloor(m.el, 'USD');
    await removeFloor(m.el, 'JPY');
    expect(floorCodes(m.el)).toEqual([]);
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(saves()[0]!.body?.floors, 'an empty list, not omitted').toEqual([]);
    expect(stored[B01]!.floors).toEqual([]);
  });

  it('a code the register does not hold is refused by the server and its message is shown', async () => {
    const m = mount();
    await settle();
    await addFloor(m.el, 'ZZZ');
    type(floorInput(m.el, 'ZZZ'), '10');
    await save(m.el);

    expect(saves().length).toBe(1);
    expect(sentFloors(saves()[0]!)).toEqual({ USD: '1000', JPY: '50000', ZZZ: '10' });
    expect(pageText()).toContain('Currency ZZZ is not in the register (test)');
    expect(stored[B01]!.version, 'nothing written').toBe(4);
  });
});

describe('EX-5 W10: Thai and English through the screen table', () => {
  it('every key this test reads is in both tables, the English one with no Thai', () => {
    for (const k of KEYS) {
      expect(say(k, 'th')).toBeTruthy();
      expect(THAI.test(say(k, 'en')), `en.${k} has no Thai`).toBe(false);
    }
  });

  it('in English the form carries no Thai and saves the same way', async () => {
    setLang('en');
    const m = mount();
    await settle();
    expect(THAI.test(text(m.el)), `no Thai in: ${text(m.el)}`).toBe(false);
    type(field(m.el, say('staleDataMinutes', 'en')), '35');
    await save(m.el, 'en');
    expect(saves().length).toBe(1);
    expect(saves()[0]!.body?.staleDataMinutes).toBe(35);
  });
});
