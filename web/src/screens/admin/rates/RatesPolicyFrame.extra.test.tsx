import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RateDraftDetailData, RateDraftLineDto } from '../../../shared/api/generated/rate_draft';
import type { RateDraftItemDto } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatesReferenceData } from '../../../shared/api/generated/rates_reference';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';
import * as table from './i18n';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  return menuFiles['./menu.ts']!.menu;
}

const V = '/api/v3/views/';
const PUBLISHED = `${V}published_rates`;
const DRAFTS = `${V}rate_drafts`;
const DRAFT = `${V}rate_draft`;
const POLICY = `${V}rate_policy`;
const PUBLICATIONS = `${V}rate_publications`;
const REFERENCE = `${V}rates_reference`;

function line(currency: string, unitSize: string, buy: string | null, sell: string | null): PublishedLineDto {
  return { currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}`, bands: [] };
}

function mainPublished(): PublishedRatesData {
  return {
    publication: { id: 'pub-7', version: 7, publishedAt: '2026-10-09T02:00:00+00:00' },
    sourceDate: '2026-10-08',
    stale: false,
    lines: [
      line('EUR', '1', '37.35520000', '38.08440000'),
      line('JPY', '100', '23.50000000', '24.12345678'),
      line('USD', '1', '33.25000000', '34.10000000'),
    ],
    bandHeadings: [],
  };
}

function policyOf(): RatePolicyData {
  return {
    policy: {
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: {
        USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
        JPY: { buy: { min_pct: '-2.0000', max_pct: '0.5000' }, sell: { min_pct: '-0.5000', max_pct: '2.0000' } },
      },
      maxReferenceAgeHours: 24,
      onStale: 'STOP',
      staleLimitThb: null,
      validFrom: '2026-10-08T00:00:00+00:00',
      validTo: null,
      approvedBy: 'owner-1',
      reason: 'seeded',
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    },
  };
}

interface Sent {
  path: string;
  method: string;
  body: Record<string, unknown> | undefined;
}

let sent: Sent[];
let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

async function fakeFetch(input: string, init?: RequestInit): Promise<Response> {
  const url = new URL(input, 'http://localhost');
  const method = (init?.method ?? 'GET').toUpperCase();
  const body = typeof init?.body === 'string' && init.body !== '' ? JSON.parse(init.body) : undefined;
  sent.push({ path: url.pathname, method, body });

  if (url.pathname === '/api/v3/views/me') {
    return json(200, {
      version: 1,
      changed: true,
      data: {
        kind: 'owner',
        id: 'owner-1',
        role: 'OWNER',
        branch: 'MAIN',
        name: 'Owner',
        username: 'owner',
        permissions: ['RatePolicySave'],
      },
    });
  }
  if (url.pathname === QUEUE_CARD_PATH) return json(200, queueCardUnchanged());
  if (method === 'GET') {
    if (url.pathname === PUBLISHED) return json(200, { version: 7, changed: true, data: mainPublished() });
    if (url.pathname === DRAFTS) {
      const d1: RateDraftItemDto = {
        id: 'd1',
        branchId: 'MAIN',
        status: 'DRAFT',
        version: 1,
        createdBy: 'rates-9',
        createdAt: '2026-10-09T04:00:00+00:00',
        updatedAt: '2026-10-09T04:00:00+00:00',
        evaluation: null,
      };
      return json(200, { version: 1, changed: true, data: { drafts: [d1] } });
    }
    if (url.pathname === DRAFT) {
      const detail: RateDraftDetailData = {
        draft: {
          id: 'd1',
          branchId: 'MAIN',
          status: 'DRAFT',
          version: 1,
          createdBy: 'rates-9',
          createdAt: '2026-10-09T04:00:00+00:00',
          updatedAt: '2026-10-09T04:00:00+00:00',
          evaluation: null,
        },
        lines: [{ currency: 'USD', buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null } as RateDraftLineDto],
      };
      return json(200, { version: 1, changed: true, data: detail });
    }
    if (url.pathname === POLICY) return json(200, { version: 1, changed: true, data: policyOf() });
    if (url.pathname === PUBLICATIONS) return json(200, { version: 1, changed: true, data: { publications: [] } });
    if (url.pathname === REFERENCE) {
      const ref: RatesReferenceData = { rows: [], fetchedAt: '2026-10-09T01:30:00.000Z', error: null };
      return json(200, { version: 1, changed: true, data: ref });
    }
  }
  if (url.pathname === '/api/rate-policies' && method === 'POST') {
    return json(200, { id: 'pol-2', version: 2 });
  }
  return json(500, { error: { code: 'serverError', message: 'unexpected' } });
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

function mount(): HTMLElement {
  const menu = theMenu();
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-09', to: '2026-10-09' });
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

function policyCard(el: ParentNode): HTMLElement {
  return el.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]')!.closest<HTMLElement>('.rates-section')!;
}

function guardLine(card: ParentNode, code: string): HTMLElement {
  return card.querySelector<HTMLElement>(`[data-guard-line="${code}"]`)!;
}

function input(scope: ParentNode, name: string): HTMLInputElement {
  return scope.querySelector<HTMLInputElement>(`input[name="${name}"]`)!;
}

function choose(scope: ParentNode, name: string, value: string): void {
  const sel = scope.querySelector<HTMLSelectElement>(`select[name="${name}"]`)!;
  sel.value = value;
  sel.dispatchEvent(new Event('input', { bubbles: true }));
  sel.dispatchEvent(new Event('change', { bubbles: true }));
}

function type(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function policyPosts(): Sent[] {
  return sent.filter((s) => s.path === '/api/rate-policies' && s.method === 'POST');
}

async function save(el: ParentNode): Promise<void> {
  el.querySelector<HTMLButtonElement>('button[data-testid="save-policy"]')!.click();
  await settle();
}

beforeEach(() => {
  sent = [];
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

describe('A3 policy extra cases (#3357 §9)', () => {
  it('only the buy side of a guard row filled sends buy only and leaves sell out', async () => {
    const el = mount();
    await settle();
    const eur = guardLine(policyCard(el), 'EUR');
    type(input(eur, 'guardBuyMin'), '-1.2500');
    type(input(eur, 'guardBuyMax'), '1.2500');
    await save(el);

    expect(policyPosts()).toHaveLength(1);
    const guards = policyPosts()[0]!.body!.guards as Record<string, Record<string, unknown>>;
    expect(guards.EUR).toEqual({
      buy: { min_pct: '-1.2500', max_pct: '1.2500' },
    });
    expect('sell' in guards.EUR!).toBe(false);
  });

  it('a guard value with 5 decimal places is refused on screen (guards) and sends nothing', async () => {
    const el = mount();
    await settle();
    const card = policyCard(el);
    type(input(guardLine(card, 'USD'), 'guardBuyMax'), '1.12345');
    await save(el);

    expect(policyPosts()).toEqual([]);
    const err = card.querySelector('[data-field-error="guards"]');
    expect(err).not.toBeNull();
    expect(err?.textContent?.trim()).toContain(table.th.guardRangeInvalid);
  });

  it('LIMIT 100.555 is refused on screen (staleLimitThb) and sends nothing', async () => {
    const el = mount();
    await settle();
    const card = policyCard(el);
    choose(card, 'onStale', 'LIMIT');
    await settle();
    type(input(card, 'staleLimitThb'), '100.555');
    await save(el);

    expect(policyPosts()).toEqual([]);
    const err = card.querySelector('[data-field-error="staleLimitThb"]');
    expect(err).not.toBeNull();
    expect(err?.textContent?.trim()).toBe(table.th.staleLimitRequired);
  });

  it('sharpMoveAlertPct renders inside label.form-field with a .form-label holding its text (#3326)', async () => {
    const el = mount();
    await settle();
    const card = policyCard(el);
    const sharp = card.querySelector<HTMLInputElement>('input[data-testid="sharp-move-threshold"]');
    expect(sharp).not.toBeNull();
    const fieldLabel = sharp!.closest('label.form-field');
    expect(fieldLabel).not.toBeNull();
    const textSpan = fieldLabel!.querySelector('.form-label');
    expect(textSpan?.textContent?.trim()).toBe(table.th.sharpMoveThresholdLabel);
  });
});
