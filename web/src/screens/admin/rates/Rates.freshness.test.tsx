// RF-2 (#773; design docs/design/blueprint/rates.md §4.10, PR 843): Admin Rates shows the published rates' freshness —
// the stale badge, the reference's source date and the publication's time — through the ONE shared component
// `web/src/shared/ui/RateFreshness.tsx` (its own tests: RateFreshness.test.tsx), embedded once in the published
// section, under the section heading and above the published table. The data is `published_rates`' own `stale`,
// `sourceDate` and `publication.publishedAt`, already on the wire.
// Written before the implementation by the ticket author (claude/83f682), against the Rates.tsx #792 left (main
// 2b08707). Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { fill, locale, setLang } from '../../../shared/i18n';
import { th } from '../../../shared/i18n/th';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}
const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });
const sources = import.meta.glob<string>('./Rates.tsx', { query: '?raw', import: 'default', eager: true });

const PUBLISHED_AT = '2026-10-10T03:15:00+00:00';
let published: PublishedRatesData;

function publishedOf(over: Partial<PublishedRatesData>): PublishedRatesData {
  return {
    publication: { id: 'pub-7', version: 7, publishedAt: PUBLISHED_AT },
    sourceDate: '2026-10-08',
    stale: false,
    lines: [{ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '33.25000000', sellRate: '34.10000000', lineId: 'line-USD' }],
    ...over,
  };
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function view(data: unknown): Response {
  return json(200, { version: 1, changed: true, data });
}

async function fakeFetch(input: string): Promise<Response> {
  const path = new URL(input, 'http://localhost').pathname;
  const V = '/api/v3/views/';
  if (path === `${V}me`) return view({ kind: 'owner', id: 'owner-1', role: 'OWNER', branch: 'MAIN', name: 'Owner', username: 'owner' });
  if (path === QUEUE_CARD_PATH) return json(200, queueCardUnchanged());
  if (path === `${V}published_rates`) return view(published);
  if (path === `${V}rate_drafts`) return view({ drafts: [] });
  if (path === `${V}rate_draft`) return view({ draft: null, lines: [] });
  if (path === `${V}rate_policy`) return view({ policy: null });
  if (path === `${V}rate_publications`) return view({ publications: [] });
  if (path === `${V}rates_reference`) return view({ rows: [], fetchedAt: '2026-10-10T01:30:00.000Z', error: null });
  return json(500, { error: { code: 'serverError', message: `unexpected ${path}` } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

async function mount(): Promise<HTMLElement> {
  const m = menuFiles['./menu.ts'];
  expect(m, 'rates/menu.ts exists').toBeDefined();
  const Component = m!.menu.Component;
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-10', to: '2026-10-10' });
  const [shown] = createSignal(true);
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
  await settle();
  return el;
}

/** The published section: the `.rates-section` that holds the `[data-published-line]` rows. */
function publishedSection(el: HTMLElement): HTMLElement {
  const sections = [...el.querySelectorAll<HTMLElement>('.rates-section')].filter((s) =>
    s.querySelector('[data-published-line]'),
  );
  expect(sections, 'one published section').toHaveLength(1);
  return sections[0]!;
}

function text(n: Element): string {
  return (n.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function key(k: string): string {
  return (th as Record<string, string>)[k] ?? `<missing ${k}>`;
}

function at(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

beforeEach(() => {
  published = publishedOf({});
  vi.stubGlobal('fetch', vi.fn(fakeFetch));
  toast('');
  location.hash = '#rates';
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

describe('Admin Rates shows the published rates freshness (RF-2)', () => {
  it('Rates.tsx imports the shared RateFreshness, and declares no freshness markup of its own', () => {
    const src = sources['./Rates.tsx'];
    expect(src, 'Rates.tsx').toBeTypeOf('string');
    expect(src).toMatch(/import\s*\{[^}]*\bRateFreshness\b[^}]*\}\s*from\s*['"][./]*shared\/ui\/RateFreshness['"]/);
    expect(src).not.toMatch(/xc-rate-stale|xc-rate-freshness/);
    expect(src).not.toMatch(/real[\s-]?time|เรียลไทม์/i);
  });

  it('one freshness line in the published section, above the table, under the heading', async () => {
    published = publishedOf({ stale: false });
    const el = await mount();
    const section = publishedSection(el);
    const lines = section.querySelectorAll('p.xc-rate-freshness');
    expect(lines, 'one p.xc-rate-freshness in the published section').toHaveLength(1);
    expect(el.querySelectorAll('p.xc-rate-freshness'), 'and only there on the page').toHaveLength(1);
    const p = lines[0]!;
    const table = section.querySelector('table')!;
    const heading = section.querySelector('h2')!;
    expect(heading.compareDocumentPosition(p) & Node.DOCUMENT_POSITION_FOLLOWING, 'after the heading').toBeTruthy();
    expect(p.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING, 'before the table').toBeTruthy();
  });

  it('a fresh publication: no badge, the source date and the published time', async () => {
    published = publishedOf({ stale: false, sourceDate: '2026-10-08' });
    const el = await mount();
    const p = publishedSection(el).querySelector('p.xc-rate-freshness')!;
    expect(p.querySelector('.xc-rate-stale')).toBeNull();
    expect(text(p)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-08' }));
    expect(text(p)).toContain(fill(key('rate.publishedAt'), { at: at(PUBLISHED_AT) }));
  });

  it('a stale publication: the badge', async () => {
    published = publishedOf({ stale: true, sourceDate: '2026-10-06' });
    const el = await mount();
    const p = publishedSection(el).querySelector('p.xc-rate-freshness')!;
    const badge = p.querySelector('span.xc-rate-stale');
    expect(badge, 'the stale badge').not.toBeNull();
    expect(text(badge!)).toBe(key('rate.stale'));
    expect(text(p)).toContain(fill(key('rate.sourceDate'), { date: '2026-10-06' }));
  });

  it('before the first publication: no reference text and no published time', async () => {
    published = publishedOf({ publication: null, sourceDate: null, stale: false, lines: [] });
    const el = await mount();
    const sections = [...el.querySelectorAll<HTMLElement>('.rates-section')].filter((s) =>
      s.querySelector('p.xc-rate-freshness'),
    );
    expect(sections, 'the freshness line is still shown with no publication').toHaveLength(1);
    const p = sections[0]!.querySelector('p.xc-rate-freshness')!;
    expect(text(p)).toContain(key('rate.noSourceDate'));
    const prefix = key('rate.publishedAt').split('{at}')[0]!.trim();
    expect(text(p)).not.toContain(prefix);
    expect(text(p)).not.toContain('Invalid Date');
  });
});
