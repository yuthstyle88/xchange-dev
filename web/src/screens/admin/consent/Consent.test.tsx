// CI-8 (#655): the admin consent and retention screen — the owner publishes consent texts per
// purpose and language, and sets the retention period per data type.
// Written before the implementation by the ticket author's delegate (claude/5edf7bb5, lane
// controller of xchange, for claude/89458e53). Do not edit, delete or weaken; add tests in a file
// of your own.
//
// Design: `docs/design/blueprint/customer-identity-consent.md` §0 U1 (texts are versioned data per
// purpose and language th/en/zh that EXECUTIVE enters), U2 (one period per data type; no period =
// never purged), §4.1 (`retention_policies.period_days NULL CHECK (period_days > 0)`; data types
// IDENTITY_VALUE, CONTACT, CLOSED_CUSTOMER), §4.4 (`consentTextPublish` POST /api/consent-texts,
// `retentionSet` PUT /api/retention-policies/{dataType}, view `consent_texts`), §9 CI-8 and §9.1
// CI-8 row ("EXECUTIVE sees and publishes texts; other roles do not see the screen; the retention
// form sends `period_days` or null"). Wire shapes are PR 876's (#653 CI-6) at `53060b6`:
// `api/src/presentation/http/v3/commands/consent_text_publish.rs` (body `{ purpose, lang, body }`
// → `{ purpose, lang, version }`), `.../commands/retention_set.rs` (path `{dataType}`, body
// `{ periodDays: number | null }` → `{ dataType, periodDays }`), `.../views/consent_texts.rs` and
// `web/src/shared/api/generated/consent_texts.ts` (the current text per purpose and language).
// Both commands inherit the RPC row `consentPolicy`, which only the owner reaches
// (`xc-auth/.../policy.rs`: ActorKind::Owner allows `consentPolicy`; staff need
// `ConsentPolicyManage`, held by the Blueprint's EXECUTIVE only) — the v3 web role of that actor is
// `OWNER`.
//
// Behaviour this file locks (the ticket's behaviour table):
// | case                                                    | required result                                   |
// |---------------------------------------------------------|---------------------------------------------------|
// | the menu record                                         | id `consent`, roles `['OWNER']`, not dated, once  |
// | any other role (MANAGER, BACKEND, FINANCE, RATES, ...)  | the shell does not list the menu                  |
// | the screen is shown                                     | reads view `consent_texts` with the picked branch; |
// |                                                         | each current text shows purpose, lang, version, body |
// | publish with purpose, language and a body               | one POST /api/consent-texts `{purpose,lang,body,branch}`, |
// |                                                         | its own non-empty Idempotency-Key; then re-reads  |
// | publish with an empty or blank body                     | nothing sent; shows `bodyRequired`                |
// | publish refused by the server                           | shows the server's `message`                      |
// | retention: a whole number of days > 0                   | PUT /api/retention-policies/<TYPE> `{periodDays: n}` (a number) |
// | retention: the days field left empty                    | PUT with `{periodDays: null}` (no period: never purged) |
// | retention: 0, negative or not a whole number            | nothing sent; shows `periodInvalid`               |
// | data types                                              | exactly IDENTITY_VALUE, CONTACT, CLOSED_CUSTOMER, each by its table label |
//
// Signatures, props and DOM hooks this test assumes (the implementer follows them):
// - `web/src/screens/admin/consent/menu.ts` exports `menu: AdminMenu` with id `consent` (the
//   folder name, `menus.ts` `collectMenus`), `roles: ['OWNER']`, `dated: false`, reached through
//   `menus.ts`'s glob; its texts are in `./i18n.ts` (`th` and `en`, namespace `admin.consent.*`).
// - The component builds its own `new ApiClient()`; the fake API is the global `fetch`.
// - Each current text is an element `[data-consent-text="<PURPOSE>:<lang>"]` whose text holds its
//   version number and its body.
// - The publish form: a `<select>` labelled by the table's `purpose` (option values `TERMS`,
//   `MARKETING`), a `<select>` labelled `language` (option values `th`, `en`, `zh`), a textarea
//   labelled `body`, and the button `publish` (found by whole text).
// - Publish is `ApiClient.command('POST', '/api/consent-texts', { purpose, lang, body, branch }, key)`.
// - Each data type is an element `[data-retention="<TYPE>"]` showing the table's
//   `data<TYPE>` label (never only the raw code), holding an input labelled `periodDays` and the
//   button `save`. Saving is `ApiClient.command('PUT', '/api/retention-policies/<TYPE>',
//   { periodDays, branch }, key)`.
// - Keys of `./i18n.ts` this test reads: `label`, `purpose`, `language`, `body`, `publish`,
//   `bodyRequired`, `periodDays`, `save`, `periodInvalid`, `dataIDENTITY_VALUE`, `dataCONTACT`,
//   `dataCLOSED_CUSTOMER`.
//
// The menu and its table are read through eager globs, so on a branch without them each test fails
// on an assertion that the file exists, not on the import.
//
// Amended for OR-7.7 (#1030, ruling (b) issuecomment-6105567502) by the test-writer claude/5d45b017:
// the menu record's `roles: ['OWNER']` pin → `needs: 'ConsentPolicyManage'` (`policy.rs:571`), and
// "no other role sees it" now reads the Blueprint roles' permissions (`web/src/test-support/me.ts`,
// copied from role_permissions.rs): only EXECUTIVE holds it (#1030 §3.10).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { ConsentTextViewItem } from '../../../shared/api/generated/consent_texts';
import { menus, type AdminMenu, type AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { permissionsOf } from '../../../test-support/me';
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
  expect(m, 'web/src/screens/admin/consent/menu.ts exists').toBeDefined();
  return m!.menu;
}

function texts(): TextsModule {
  const m = textFiles['./i18n.ts'];
  expect(m, 'web/src/screens/admin/consent/i18n.ts exists').toBeDefined();
  return m!;
}

const KEYS = [
  'label',
  'purpose',
  'language',
  'body',
  'publish',
  'bodyRequired',
  'periodDays',
  'save',
  'periodInvalid',
  'dataIDENTITY_VALUE',
  'dataCONTACT',
  'dataCLOSED_CUSTOMER',
] as const;
type K = (typeof KEYS)[number];

function say(key: K, lang: 'th' | 'en' = 'th'): string {
  const value = texts()[lang][key];
  expect(value, `admin.consent i18n ${lang}.${key}`).toBeTruthy();
  return value!;
}

const TEXTS_VIEW = '/api/v3/views/consent_texts';
const PUBLISH = '/api/consent-texts';
const RETENTION = /^\/api\/retention-policies\/([^/]+)$/;
const THAI = /[฀-๿]/;
const DATA_TYPES = ['IDENTITY_VALUE', 'CONTACT', 'CLOSED_CUSTOMER'] as const;

// ── the fake API ─────────────────────────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body: Record<string, unknown> | undefined;
}
type Reply = { status: number; body: unknown };

let sent: Sent[];
let published: ConsentTextViewItem[];
let textsVersion: number;
let publishRefusal: { status: number; code: string; message: string } | null;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function current(): ConsentTextViewItem[] {
  const newest = new Map<string, ConsentTextViewItem>();
  for (const t of published) {
    const k = `${t.purpose}:${t.lang}`;
    const seen = newest.get(k);
    if (!seen || seen.version < t.version) newest.set(k, t);
  }
  return [...newest.values()].sort((a, b) => `${a.purpose}:${a.lang}`.localeCompare(`${b.purpose}:${b.lang}`));
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
  if (path === TEXTS_VIEW && method === 'GET') {
    const since = p.get('since');
    if (since !== null && Number(since) === textsVersion) {
      return { status: 200, body: { version: textsVersion, changed: false } };
    }
    return { status: 200, body: { version: textsVersion, changed: true, data: { texts: structuredClone(current()) } } };
  }
  if (path === PUBLISH && method === 'POST') {
    if (publishRefusal) {
      return {
        status: publishRefusal.status,
        body: { error: { code: publishRefusal.code, message: publishRefusal.message } },
      };
    }
    const purpose = String(body?.purpose);
    const lang = String(body?.lang);
    const version = 1 + Math.max(0, ...published.filter((t) => t.purpose === purpose && t.lang === lang).map((t) => t.version));
    published.push({ purpose, lang, version, body: String(body?.body), publishedAt: '2026-10-10T07:00:00+00:00' });
    textsVersion += 1;
    return { status: 200, body: { purpose, lang, version } };
  }
  const r = RETENTION.exec(path);
  if (r && method === 'PUT') {
    return { status: 200, body: { dataType: r[1], periodDays: body?.periodDays ?? null } };
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
  return sent.filter((s) => s.path === TEXTS_VIEW && s.method === 'GET');
}
function writes(): Sent[] {
  return sent.filter((s) => s.method !== 'GET' && !PAGE_WIDE.includes(s.path));
}
function publishes(): Sent[] {
  return writes().filter((s) => s.path === PUBLISH);
}
function retentionWrites(): Sent[] {
  return writes().filter((s) => RETENTION.test(s.path));
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
  const [period] = createSignal({ from: '2026-10-10', to: '2026-10-10' });
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
  textsVersion = 1;
  published = [
    { purpose: 'TERMS', lang: 'th', version: 1, body: 'terms th v1', publishedAt: '2026-10-01T00:00:00+00:00' },
    { purpose: 'TERMS', lang: 'th', version: 2, body: 'terms th v2', publishedAt: '2026-10-02T00:00:00+00:00' },
    { purpose: 'TERMS', lang: 'en', version: 1, body: 'terms en v1', publishedAt: '2026-10-02T00:00:00+00:00' },
    { purpose: 'MARKETING', lang: 'th', version: 3, body: 'marketing th v3', publishedAt: '2026-10-03T00:00:00+00:00' },
  ];
  publishRefusal = null;
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#consent';
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

function button(el: ParentNode, name: string): HTMLButtonElement {
  const found = [...el.querySelectorAll<HTMLButtonElement>('button')].filter((b) => text(b) === name);
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

function optionValues(select: Control): string[] {
  expect(select.tagName, 'a <select>').toBe('SELECT');
  return [...(select as HTMLSelectElement).options].map((o) => o.value).filter((v) => v !== '');
}

async function publish(el: HTMLElement, purpose: string, lang: string, body: string): Promise<void> {
  type(field(el, say('purpose')), purpose);
  type(field(el, say('language')), lang);
  type(field(el, say('body')), body);
  button(el, say('publish')).click();
  await settle();
}

async function setRetention(el: HTMLElement, dataType: string, days: string): Promise<void> {
  const row = one(el, `[data-retention="${dataType}"]`);
  type(field(row, say('periodDays')), days);
  button(row, say('save')).click();
  await settle();
}

// ── the menu record and the role gate ────────────────────────────────────────

describe('CI-8: the consent menu record (#655; design §4.4, §9.1)', () => {
  it('is the consent menu: needs ConsentPolicyManage, not dated, reached through the shell glob once', () => {
    const menu = theMenu();
    expect(menu.id).toBe('consent');
    expect(menu.needs, 'consentPolicy needs ConsentPolicyManage (policy.rs:571)').toBe('ConsentPolicyManage');
    expect(menu.dated).toBe(false);
    expect(typeof menu.Component).toBe('function');
    const listed = menus.filter((m) => m.id === 'consent');
    expect(listed.length).toBe(1);
    expect(listed[0]).toBe(menu);
  });

  it('no other role sees it — of the Blueprint roles only EXECUTIVE holds what the menu needs', () => {
    const menu = theMenu();
    expect(menu.needs).not.toBe('all');
    for (const role of ['ADMIN', 'RATE_MANAGER', 'BRANCH_MANAGER', 'TELLER', 'FINANCE', 'MARKETING', 'VERIFICATION_REVIEWER']) {
      expect(permissionsOf([{ role, branchId: null }]).includes(menu.needs as string), `role ${role} must not list the consent menu`).toBe(
        false,
      );
    }
    expect(permissionsOf([{ role: 'EXECUTIVE', branchId: null }])).toContain(menu.needs);
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
    setLang('th');
    expect(menu.label()).toBe(say('label', 'th'));
    setLang('en');
    expect(menu.label()).toBe(say('label', 'en'));
  });

  it('goes through ApiClient, never a bare fetch', () => {
    const own = Object.entries(sources);
    expect(own.length, 'the consent folder has source files').toBeGreaterThan(0);
    for (const [file, src] of own) {
      expect(/\bfetch\s*\(/.test(src), `${file} calls fetch directly`).toBe(false);
    }
  });
});

// ── the texts ────────────────────────────────────────────────────────────────

describe('CI-8: the current consent texts (U1; view consent_texts)', () => {
  it('reads consent_texts for the picked branch and shows each current text with its version and body', async () => {
    const el = mount({ branch: 'B9' });
    await settle();
    expect(reads().length, 'consent_texts read on show').toBeGreaterThan(0);
    expect(reads()[0]!.params.get('branch')).toBe('B9');

    const termsTh = one(el, '[data-consent-text="TERMS:th"]');
    expect(text(termsTh)).toContain('terms th v2');
    expect(text(termsTh)).toContain('2');
    expect(text(termsTh), 'only the current version is shown').not.toContain('terms th v1');
    expect(text(one(el, '[data-consent-text="TERMS:en"]'))).toContain('terms en v1');
    const mk = one(el, '[data-consent-text="MARKETING:th"]');
    expect(text(mk)).toContain('marketing th v3');
    expect(text(mk)).toContain('3');
    expect(el.querySelectorAll('[data-consent-text]').length).toBe(3);
  });

  it('offers exactly the two purposes and the three languages', async () => {
    const el = mount();
    await settle();
    expect(optionValues(field(el, say('purpose'))).sort()).toEqual(['MARKETING', 'TERMS']);
    expect(optionValues(field(el, say('language'))).sort()).toEqual(['en', 'th', 'zh']);
  });
});

describe('CI-8: publishing a text (consentTextPublish, §4.4)', () => {
  it('sends one POST /api/consent-texts with purpose, lang, body and branch under its own key, then re-reads', async () => {
    const el = mount();
    await settle();
    const before = reads().length;

    await publish(el, 'MARKETING', 'zh', 'marketing zh v1');

    expect(publishes().length).toBe(1);
    const p = publishes()[0]!;
    expect(p.method).toBe('POST');
    expect(p.body).toEqual({ purpose: 'MARKETING', lang: 'zh', body: 'marketing zh v1', branch: 'MAIN' });
    expect(p.key, 'Idempotency-Key').toBeTruthy();
    expect(reads().length, 'the texts are read again after the publish').toBeGreaterThan(before);
    expect(text(one(el, '[data-consent-text="MARKETING:zh"]'))).toContain('marketing zh v1');
  });

  it('a new version of an existing text replaces the shown one', async () => {
    const el = mount();
    await settle();
    await publish(el, 'TERMS', 'th', 'terms th v3');
    const shown = one(el, '[data-consent-text="TERMS:th"]');
    expect(text(shown)).toContain('terms th v3');
    expect(text(shown)).toContain('3');
    expect(text(shown)).not.toContain('terms th v2');
  });

  it('two publishes carry two different keys', async () => {
    const el = mount();
    await settle();
    await publish(el, 'TERMS', 'en', 'terms en v2');
    await publish(el, 'TERMS', 'zh', 'terms zh v1');
    expect(publishes().length).toBe(2);
    expect(publishes()[0]!.key).toBeTruthy();
    expect(publishes()[1]!.key).toBeTruthy();
    expect(publishes()[0]!.key).not.toBe(publishes()[1]!.key);
  });

  it('an empty or blank body sends nothing and says the body is required', async () => {
    const el = mount();
    await settle();
    await publish(el, 'TERMS', 'th', '');
    await publish(el, 'TERMS', 'th', '   ');
    expect(publishes().length).toBe(0);
    expect(text(el)).toContain(say('bodyRequired'));
  });

  it('a refused publish shows the server message', async () => {
    publishRefusal = { status: 403, code: 'roleNotAllowedHere', message: 'refused: not the owner' };
    const el = mount();
    await settle();
    await publish(el, 'TERMS', 'en', 'terms en v2');
    expect(publishes().length).toBe(1);
    expect(text(document.body)).toContain('refused: not the owner');
  });
});

// ── retention ────────────────────────────────────────────────────────────────

describe('CI-8: the retention form (retentionSet, U2, §4.1)', () => {
  it('lists exactly the three data types, each by its table label', async () => {
    const el = mount();
    await settle();
    const rows = [...el.querySelectorAll<HTMLElement>('[data-retention]')].map((r) => r.dataset.retention);
    expect(rows.sort()).toEqual([...DATA_TYPES].sort());
    for (const t of DATA_TYPES) {
      expect(text(one(el, `[data-retention="${t}"]`))).toContain(say(`data${t}` as K));
    }
  });

  it('a whole number of days is sent as a number to that data type', async () => {
    const el = mount();
    await settle();
    await setRetention(el, 'IDENTITY_VALUE', '30');
    expect(retentionWrites().length).toBe(1);
    const w = retentionWrites()[0]!;
    expect(w.method).toBe('PUT');
    expect(w.path).toBe('/api/retention-policies/IDENTITY_VALUE');
    expect(w.body?.periodDays, 'periodDays is a number, not a string').toBe(30);
    expect(w.body?.branch).toBe('MAIN');
    expect(w.key, 'Idempotency-Key').toBeTruthy();
  });

  it('an empty field sends periodDays null — no period, never purged', async () => {
    const el = mount();
    await settle();
    await setRetention(el, 'CONTACT', '');
    expect(retentionWrites().length).toBe(1);
    const w = retentionWrites()[0]!;
    expect(w.path).toBe('/api/retention-policies/CONTACT');
    expect(w.body).toHaveProperty('periodDays');
    expect(w.body?.periodDays).toBeNull();
  });

  it('0, a negative or a fractional number of days sends nothing and says the period is invalid', async () => {
    const el = mount();
    await settle();
    for (const bad of ['0', '-5', '1.5', 'abc']) {
      await setRetention(el, 'CLOSED_CUSTOMER', bad);
    }
    expect(retentionWrites().length).toBe(0);
    expect(text(el)).toContain(say('periodInvalid'));
  });

  it('two saves carry two different keys', async () => {
    const el = mount();
    await settle();
    await setRetention(el, 'IDENTITY_VALUE', '365');
    await setRetention(el, 'CLOSED_CUSTOMER', '730');
    expect(retentionWrites().length).toBe(2);
    expect(retentionWrites()[0]!.key).not.toBe(retentionWrites()[1]!.key);
    expect(retentionWrites()[1]!.path).toBe('/api/retention-policies/CLOSED_CUSTOMER');
    expect(retentionWrites()[1]!.body?.periodDays).toBe(730);
  });
});
