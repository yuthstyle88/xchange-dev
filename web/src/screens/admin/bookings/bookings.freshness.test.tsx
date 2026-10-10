// RF-4 #943 (design docs/design/blueprint/rates.md §4.10, row RF-4): the Admin Bookings "create" dialog shows the
// freshness of the counter rates it offers — the stale badge, the reference's source date and the publication's
// time — through the ONE shared component `web/src/shared/ui/RateFreshness.tsx` (RF-2; its own tests:
// RateFreshness.test.tsx), embedded ONCE in the dialog. The data is the `admin_booking_settings` view's new
// `data.freshness` (`RateFreshnessDto`: `{ stale, sourceDate, publishedAt }`, RF-1), passed through unchanged.
// Written before the implementation by claude/dell-claude-4 on the lane controller's order (ticket author's work).
// Do not edit, delete or weaken; add tests in a file of your own.
//
// What is pinned (ticket §3 item 3, §9):
// 1. BookingsScreen.tsx imports the shared RateFreshness and declares no freshness markup of its own.
// 2. Exactly one `p.xc-rate-freshness` in the open create dialog. Where in the dialog is the implementer's ([proposed]).
// 3. It shows the view's `freshness`: the badge only when `stale`; `rate.sourceDate` with the date; `rate.publishedAt`
//    with the time; `rate.noSourceDate` and no time when the branch has no publication.
// 4. The dialog never says "real time", "live" or "เรียลไทม์" (§05: no display implies real time), in Thai or English.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal, type JSX } from 'solid-js';
import { fill, locale, setLang } from '../../../shared/i18n';
import { en } from '../../../shared/i18n/en';
import { th } from '../../../shared/i18n/th';
import type { AdminMenu } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast } from '../parts/toast';

const loadedMenu = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const sources = import.meta.glob<string>('./BookingsScreen.tsx', {
  query: '?raw',
  import: 'default',
  eager: true,
});

const BOOKINGS = 'admin_bookings';
const SETTINGS = 'admin_booking_settings';
const BRANCHES = 'admin_branches';
const IGNORED = ['/api/v3/views/me', '/api/v3/views/kiosk_notifications'];
const PUBLISHED_AT = '2026-10-10T03:15:00.123Z';

/** `RateFreshnessDto` on the wire (RF-1, `dto/v3/common.rs`). Declared here so this file does not wait on codegen. */
interface Freshness {
  stale: boolean;
  sourceDate: string | null;
  publishedAt: string | null;
}

interface Settings {
  rules: { enabled: boolean; minutes: number };
  memberCount: number;
  publicUrl: string;
  counterRates: { currency: string; buy: string; sell: string }[];
  freshness: Freshness;
}

const RATES = [
  { currency: 'EUR', buy: '39.5000', sell: '40.1000' },
  { currency: 'USD', buy: '36.2500', sell: '36.7500' },
];

function settingsWith(freshness: Freshness, counterRates = RATES): Settings {
  return { rules: { enabled: true, minutes: 30 }, memberCount: 0, publicUrl: '', counterRates, freshness };
}

let settings: Settings;

function respond(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const url = new URL(String(input), 'http://desk.test');
  const method = (init?.method ?? 'GET').toUpperCase();
  if (IGNORED.includes(url.pathname)) {
    return respond(200, {
      version: 1,
      changed: true,
      data: { kind: 'owner', id: 'owner', role: 'OWNER', branch: 'MAIN' },
    });
  }
  if (method === 'GET' && url.pathname.startsWith('/api/v3/views/')) {
    const view = url.pathname.slice('/api/v3/views/'.length);
    if (view === BOOKINGS) return respond(200, { version: 7, changed: true, data: [] });
    if (view === SETTINGS) return respond(200, { version: 7, changed: true, data: structuredClone(settings) });
    if (view === BRANCHES) {
      return respond(200, { version: 7, changed: true, data: { branches: [{ id: 'MAIN', name: 'XC-B' }] } });
    }
  }
  return respond(404, { error: { code: 'notFound', message: `no ${method} ${url.pathname}` } });
}

let dispose: (() => void) | undefined;
let container: HTMLDivElement;

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

function norm(s: string | null | undefined): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

async function mounted(): Promise<void> {
  const m = loadedMenu['./menu.ts'];
  expect(m?.menu, 'web/src/screens/admin/bookings/menu.ts exports `menu`').toBeDefined();
  const Menu = m!.menu.Component;
  const [branch] = createSignal('MAIN');
  const [shown] = createSignal(true);
  const period = () => ({ from: '', to: '' });
  container = document.createElement('div');
  document.body.appendChild(container);
  dispose = render(
    () =>
      (
        <>
          <Menu branch={branch} period={period} shown={shown} />
          <Dialog />
          <Toast />
        </>
      ) as JSX.Element,
    container,
  );
  await settle();
}

/** Clicks the list header's create button (สร้างการจอง / its English) and answers the open dialog. */
async function openCreate(label = 'สร้างการจอง'): Promise<HTMLDialogElement> {
  const found = [...container.querySelectorAll('button')].filter(
    (b) => !b.closest('dialog') && norm(b.textContent).includes(label),
  );
  expect(found.length, `one create button "${label}"`).toBe(1);
  found[0]!.click();
  await settle();
  const d = container.querySelector('dialog');
  expect(d?.open, 'the create dialog is open').toBe(true);
  return d!;
}

/** The dialog's one freshness line. */
function theLine(d: HTMLDialogElement): HTMLElement {
  const lines = d.querySelectorAll<HTMLElement>('p.xc-rate-freshness');
  expect(lines, 'one p.xc-rate-freshness in the create dialog').toHaveLength(1);
  return lines[0]!;
}

function key(k: string): string {
  return (th as Record<string, string>)[k] ?? `<missing ${k}>`;
}

function at(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

const REAL_TIME = /real[\s-]?time|\blive\b|เรียลไทม์/i;

beforeEach(() => {
  setLang('th');
  settings = settingsWith({ stale: false, sourceDate: '2026-10-09', publishedAt: PUBLISHED_AT });
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
});

afterEach(async () => {
  closeDialog();
  dispose?.();
  dispose = undefined;
  container?.remove();
  vi.unstubAllGlobals();
  setLang('th');
  await settle();
});

describe('the Admin Bookings create dialog shows the counter rates’ freshness (RF-4 #943)', () => {
  it('BookingsScreen.tsx imports the shared RateFreshness, and declares no freshness markup of its own', () => {
    const src = sources['./BookingsScreen.tsx'];
    expect(src, 'BookingsScreen.tsx').toBeTypeOf('string');
    expect(src).toMatch(/import\s*\{[^}]*\bRateFreshness\b[^}]*\}\s*from\s*['"][./]*shared\/ui\/RateFreshness['"]/);
    expect(src).toMatch(/<RateFreshness\b/);
    expect(src).not.toMatch(/xc-rate-stale|xc-rate-freshness/);
    expect(src).not.toMatch(REAL_TIME);
  });

  it('a fresh publication: one line in the dialog, no badge, the source date and the published time', async () => {
    await mounted();
    const d = await openCreate();
    const p = theLine(d);
    expect(p.querySelector('.xc-rate-stale'), 'no badge when not stale').toBeNull();
    expect(norm(p.textContent)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-09' }));
    expect(norm(p.textContent)).toContain(fill(key('rate.publishedAt'), { at: at(PUBLISHED_AT) }));
  });

  it('a stale publication: the badge and the oldest reference date', async () => {
    settings = settingsWith({ stale: true, sourceDate: '2026-10-06', publishedAt: PUBLISHED_AT });
    await mounted();
    const d = await openCreate();
    const p = theLine(d);
    const badge = p.querySelector('span.xc-rate-stale');
    expect(badge, 'the stale badge').not.toBeNull();
    expect(norm(badge!.textContent)).toBe(key('rate.stale'));
    expect(norm(p.textContent)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-06' }));
    expect(norm(p.textContent)).toContain(fill(key('rate.publishedAt'), { at: at(PUBLISHED_AT) }));
  });

  it('MANUAL with no reference: no badge, rate.noSourceDate, and the published time', async () => {
    settings = settingsWith({ stale: false, sourceDate: null, publishedAt: PUBLISHED_AT });
    await mounted();
    const d = await openCreate();
    const p = theLine(d);
    expect(p.querySelector('.xc-rate-stale')).toBeNull();
    expect(norm(p.textContent)).toContain(key('rate.noSourceDate'));
    expect(norm(p.textContent)).toContain(fill(key('rate.publishedAt'), { at: at(PUBLISHED_AT) }));
  });

  it('no publication for the branch: no badge, no reference text and no published time', async () => {
    settings = settingsWith({ stale: false, sourceDate: null, publishedAt: null }, []);
    await mounted();
    const d = await openCreate();
    const p = theLine(d);
    expect(p.querySelector('.xc-rate-stale')).toBeNull();
    expect(norm(p.textContent)).toContain(key('rate.noSourceDate'));
    const prefix = key('rate.publishedAt').split('{at}')[0]!.trim();
    expect(norm(p.textContent)).not.toContain(prefix);
    expect(norm(p.textContent)).not.toContain('Invalid Date');
  });

  it('the dialog never says real time, live or เรียลไทม์, in Thai or in English', async () => {
    settings = settingsWith({ stale: true, sourceDate: '2026-10-06', publishedAt: PUBLISHED_AT });
    await mounted();
    let d = await openCreate();
    theLine(d);
    expect(norm(d.textContent)).not.toMatch(REAL_TIME);
    closeDialog();
    await settle();

    setLang('en');
    await settle();
    d = await openCreate('Create booking');
    const p = theLine(d);
    expect(norm(p.textContent)).toContain((en as Record<string, string>)['rate.stale']);
    expect(norm(d.textContent)).not.toMatch(REAL_TIME);
  });
});
