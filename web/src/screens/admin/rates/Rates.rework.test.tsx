import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type { RateDraftDetailData, RateDraftLineDto } from '../../../shared/api/generated/rate_draft';
import type { RateDraftItemDto } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatesReferenceData, RatesReferenceRow } from '../../../shared/api/generated/rates_reference';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog } from '../parts/Dialog';
import { Toast, toast } from '../parts/toast';
import { QUEUE_CARD_PATH, queueCardUnchanged } from '../../../shared/testing/page-wide';

interface MenuModule {
  menu: AdminMenu & { Component: (props: AdminMenuProps) => JSX.Element };
}

const menuFiles = import.meta.glob<MenuModule>('./menu.ts', { eager: true });

function theMenu(): MenuModule['menu'] {
  const m = menuFiles['./menu.ts'];
  expect(m, 'web/src/screens/admin/rates/menu.ts exists').toBeDefined();
  return m!.menu;
}

const PUBLISHED = '/api/v3/views/published_rates';
const DRAFTS = '/api/v3/views/rate_drafts';
const DRAFT = '/api/v3/views/rate_draft';
const POLICY = '/api/v3/views/rate_policy';
const PUBLICATIONS = '/api/v3/views/rate_publications';
const REFERENCE = '/api/v3/views/rates_reference';

function line(currency: string, unitSize: string, buy: string | null, sell: string | null): PublishedLineDto {
  return { currency, group: 'ALL', unitSize, buyRate: buy, sellRate: sell, lineId: `line-${currency}` };
}

function publishedOf(version: number | null, lines: PublishedLineDto[]): PublishedRatesData {
  return {
    publication: version === null ? null : { id: `pub-${version}`, version, publishedAt: '2026-10-09T02:00:00+00:00' },
    sourceDate: '2026-10-08',
    stale: false,
    lines,
  };
}

function mainPublished(): PublishedRatesData {
  return publishedOf(7, [
    line('EUR', '1', '37.35520000', '38.08440000'),
    line('USD', '1', '33.25000000', '34.10000000'),
  ]);
}

function draftItem(id: string, status: string, version: number, createdAt: string): RateDraftItemDto {
  return {
    id,
    branchId: 'MAIN',
    status,
    version,
    createdBy: 'rates-9',
    createdAt,
    updatedAt: createdAt,
    evaluation: null,
  };
}

function auto(currency: string): RateDraftLineDto {
  return { currency, buyMode: 'AUTO', sellMode: 'AUTO', manualBuy: null, manualSell: null, reason: null };
}

function detailOf(item: RateDraftItemDto, lines: RateDraftLineDto[]): RateDraftDetailData {
  return { draft: { ...item }, lines };
}

const GUARDS = {
  USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' }, sell: { min_pct: '-1.0000', max_pct: '1.0000' } },
};

function policyOf(): RatePolicyData {
  return {
    policy: {
      id: 'pol-1',
      branchId: 'MAIN',
      version: 1,
      buyAdjustPct: '0.0000',
      sellAdjustPct: '0.0000',
      guards: structuredClone(GUARDS),
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

function refRow(code: string, unit: number, sight: string, transfer: string, selling: string): RatesReferenceRow {
  return { code, date: '2026-10-08', unit, buyingSight: sight, buyingTransfer: transfer, selling };
}

function referenceOf(): RatesReferenceData {
  return {
    rows: [refRow('USD', 1, '33.4218', '33.5008', '33.8278')],
    fetchedAt: '2026-10-09T01:30:00.000Z',
    error: null,
  };
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
let publishedReply: (() => Reply) | null;
let draftsReply: (() => Reply) | null;
let draftDetailReply: (() => Reply) | null;
let policyReply: (() => Reply) | null;
let referenceReply: (() => Reply) | null;
let publicationsReply: (() => Reply) | null;
let commandReplies: Record<string, () => Reply>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function refusal(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

async function answer(s: Sent): Promise<Reply> {
  const { path, method } = s;
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

  if (method === 'GET') {
    if (path === PUBLISHED) {
      if (publishedReply) return publishedReply();
      return { status: 200, body: { version: 1, changed: true, data: mainPublished() } };
    }
    if (path === DRAFTS) {
      if (draftsReply) return draftsReply();
      const d1 = draftItem('d1', 'DRAFT', 1, '2026-10-09T04:00:00+00:00');
      return { status: 200, body: { version: 1, changed: true, data: { drafts: [d1] } } };
    }
    if (path === DRAFT) {
      if (draftDetailReply) return draftDetailReply();
      const d1 = draftItem('d1', 'DRAFT', 1, '2026-10-09T04:00:00+00:00');
      return { status: 200, body: { version: 1, changed: true, data: detailOf(d1, [auto('EUR'), auto('USD')]) } };
    }
    if (path === POLICY) {
      if (policyReply) return policyReply();
      return { status: 200, body: { version: 1, changed: true, data: policyOf() } };
    }
    if (path === REFERENCE) {
      if (referenceReply) return referenceReply();
      return { status: 200, body: { version: 1, changed: true, data: referenceOf() } };
    }
    if (path === PUBLICATIONS) {
      if (publicationsReply) return publicationsReply();
      return { status: 200, body: { version: 1, changed: true, data: { publications: [] } } };
    }
    return refusal(500, 'serverError', `unexpected read ${path}`);
  }

  const customCmd = commandReplies[`${method} ${path}`];
  if (customCmd) return customCmd();

  if (path === '/api/rate-references/fetch' && method === 'POST') return { status: 200, body: { batchId: 'batch-1' } };
  if (path === '/api/rate-policies' && method === 'POST') return { status: 200, body: { id: 'pol-2', version: 2 } };
  if (path === '/api/rate-drafts' && method === 'POST') {
    return { status: 200, body: { id: 'd-new-1', version: 1, status: 'DRAFT' } };
  }
  if (path === '/api/rate-drafts/d1/simulate' && method === 'POST') {
    return { status: 200, body: { id: 'd1', isInBound: true, outOfBound: [] } };
  }
  if (path === '/api/rate-drafts/d1/submit' && method === 'POST') {
    return { status: 200, body: { requestId: 'req-1', status: 'PENDING_APPROVAL' } };
  }
  if (path === '/api/rate-drafts/d1/publish' && method === 'POST') {
    return { status: 200, body: { id: 'pub-8', version: 8 } };
  }
  if (path === '/api/rate-drafts/d1' && method === 'PUT') {
    return { status: 200, body: { id: 'd1', version: 2, status: 'DRAFT' } };
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
  const reply = await answer(s);
  return json(reply.status, reply.body);
}

async function settle(): Promise<void> {
  for (let round = 0; round < 4; round++) {
    for (let i = 0; i < 5; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
  }
  for (let i = 0; i < 5; i++) await Promise.resolve();
}

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

interface Mounted {
  el: HTMLElement;
  setBranch: (b: string) => void;
}

function mount(init: { branch?: string } = {}): Mounted {
  const menu = theMenu();
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
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
  return { el, setBranch };
}

function toastText(): string | null {
  const t = document.querySelector('.toast');
  if (!t || t.hasAttribute('hidden')) return null;
  return t.textContent?.trim() ?? null;
}

beforeEach(() => {
  sent = [];
  publishedReply = null;
  draftsReply = null;
  draftDetailReply = null;
  policyReply = null;
  referenceReply = null;
  publicationsReply = null;
  commandReplies = {};
  vi.stubGlobal('fetch', fakeFetch);
  setLang('th');
  toast('');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  root = undefined;
  vi.unstubAllGlobals();
});

describe('108-Plaza/xchange#1687: Rates screen worded errors and policy guard', () => {
  // Row 1: Rates.tsx:91 - published rates fail -> worded error state
  describe('Row 1: published rates load failure shows worded error', () => {
    it('shows worded error on 500 server error', async () => {
      publishedReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const m = mount();
      await settle();

      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);
    });

    it('shows worded error on 403 refusal (roleNotAllowedHere)', async () => {
      publishedReply = () => refusal(403, 'roleNotAllowedHere', 'สาขานี้อยู่นอกสิทธิ์ของคุณ');
      const m = mount();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);
      expect(alerts.some((a) => a.textContent?.includes('สาขานี้อยู่นอกสิทธิ์ของคุณ'))).toBe(false);
    });

    it("clears the previous branch's published rates when the read on the new branch fails", async () => {
      let callCount = 0;
      publishedReply = () => {
        callCount++;
        if (callCount === 1) {
          return { status: 200, body: { version: 1, changed: true, data: mainPublished() } };
        }
        return refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      };

      const m = mount();
      await settle();

      // First read succeeded: EUR line present
      expect(m.el.querySelector('[data-published-line="EUR"]')).not.toBeNull();

      // Trigger branch change to B9 where published read fails
      m.setBranch('B9');
      await settle();

      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);
      // Previous branch rates must not remain visible after branch switch (Done criterion 3)
      expect(m.el.querySelector('[data-published-line="EUR"]')).toBeNull();
    });
  });

  // Row 2: Rates.tsx:117 - drafts fail -> worded error state
  describe('Row 2: drafts load failure shows worded error', () => {
    it('shows worded error when rate_drafts fails with 500', async () => {
      draftsReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const m = mount();
      await settle();

      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);
    });

    it('shows worded error when rate_draft detail fails with 403', async () => {
      draftDetailReply = () => refusal(403, 'roleNotAllowedHere', 'สาขานี้อยู่นอกสิทธิ์ของคุณ');
      const m = mount();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);
      expect(alerts.some((a) => a.textContent?.includes('สาขานี้อยู่นอกสิทธิ์ของคุณ'))).toBe(false);
    });
  });

  // Row 3: Rates.tsx:132 - policy fails silently -> worded error state and 'adjust all' disabled
  describe('Row 3: policy load failure shows worded error and disables adjust all button', () => {
    it('shows worded error and disables adjust all when rate_policy fails', async () => {
      policyReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const m = mount();
      await settle();

      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const alerts = Array.from(m.el.querySelectorAll('[role="alert"]'));
      const found = alerts.some((a) => a.textContent?.includes(expected));
      expect(found, `Expected error alert containing "${expected}"`).toBe(true);

      const adjustAllBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      expect(adjustAllBtn, 'Adjust all button exists when draft is open').not.toBeNull();
      expect(adjustAllBtn!.disabled, 'Adjust all button must be disabled when policy failed').toBe(true);
    });
  });

  // Row 4: Rates.tsx:343-347 - missing policy replaced by literal defaults -> refuse to save until loaded
  describe('Row 4: refuses to save when policy has not loaded', () => {
    it('does not POST default policy when policy is missing/failed', async () => {
      policyReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const m = mount();
      await settle();

      const adjustAllBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      expect(adjustAllBtn).not.toBeNull();

      // Attempt to click the adjust all button
      adjustAllBtn!.click();
      await settle();

      // Verify no POST /api/rate-policies was sent
      const policyWrites = sent.filter((s) => s.path === '/api/rate-policies' && s.method === 'POST');
      expect(policyWrites.length, 'Must refuse to save policy when policy has not loaded').toBe(0);
    });

    it('sends policy fields through unchanged when maxReferenceAgeHours and onStale are null', async () => {
      policyReply = () => ({
        status: 200,
        body: {
          version: 1,
          changed: true,
          data: {
            policy: {
              id: 'pol-null',
              branchId: 'MAIN',
              version: 1,
              buyAdjustPct: '0.0000',
              sellAdjustPct: '0.0000',
              guards: { USD: { buy: { min_pct: '-1.0000', max_pct: '1.0000' } } },
              maxReferenceAgeHours: null,
              onStale: null,
              staleLimitThb: null,
              validFrom: '2026-10-08T00:00:00+00:00',
              validTo: null,
              approvedBy: 'owner-1',
              reason: null,
              valueTiersOn: false,
              valueTiers: [],
              sharpMoveAlertPct: null,
            },
          },
        },
      });
      const m = mount();
      await settle();

      const adjBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      expect(adjBtn).not.toBeNull();
      adjBtn!.click();
      await settle();

      const policyWrite = sent.find((s) => s.path === '/api/rate-policies' && s.method === 'POST');
      expect(policyWrite, 'POST /api/rate-policies was sent').toBeDefined();
      const body = policyWrite!.body as Record<string, unknown>;
      expect(body.maxReferenceAgeHours).toBeNull();
      expect(body.onStale).toBeNull();
      expect(body.staleLimitThb).toBeNull();
      expect(body.reason).toBe('ปรับทั้งหมด');
    });
  });

  // Row 5: Rates.tsx:74-78, 257-380 - raw err.message / String(err) -> errorText lookup
  describe('Row 5: uses errorText lookup instead of raw server message', () => {
    it('uses errorText in fetchReference (.bot-error)', async () => {
      referenceReply = () => refusal(403, 'roleNotAllowedHere', 'raw unworded server error');
      const m = mount();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw unworded server error');
      const botErr = m.el.querySelector('.bot-error');
      expect(botErr, '.bot-error element exists').not.toBeNull();
      expect(botErr!.textContent).toBe(expected);
      expect(botErr!.textContent).not.toBe('raw unworded server error');
    });

    it('uses errorText in command failure toast (newDraft)', async () => {
      commandReplies['POST /api/rate-drafts'] = () => refusal(403, 'roleNotAllowedHere', 'raw draft error');
      const m = mount();
      await settle();

      const newDraftBtn = m.el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      expect(newDraftBtn).not.toBeNull();
      newDraftBtn!.click();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw draft error');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toBe('raw draft error');
    });

    it('shows the server sentence for an unknown refusal code', async () => {
      commandReplies['POST /api/rate-drafts'] = () => refusal(400, 'somethingNew', 'raw wire message');
      const m = mount();
      await settle();

      const newDraftBtn = m.el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      expect(newDraftBtn).not.toBeNull();
      newDraftBtn!.click();
      await settle();

      const expected = 'raw wire message';
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toContain('somethingNew');
    });

    it('shows generic worded error when fetch rejects with TypeError', async () => {
      commandReplies['POST /api/rate-drafts'] = () => {
        throw new TypeError('Failed to fetch');
      };
      const m = mount();
      await settle();

      const newDraftBtn = m.el.querySelector('.btn-new-draft') as HTMLButtonElement | null;
      expect(newDraftBtn).not.toBeNull();
      newDraftBtn!.click();
      await settle();

      const expected = errorText('network', 'เชื่อมต่อเซิร์ฟเวอร์ไม่ได้ กรุณาลองใหม่');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toContain('Failed to fetch');
    });

    it('uses errorText in command failure toast (simulate)', async () => {
      commandReplies['POST /api/rate-drafts/d1/simulate'] = () => refusal(403, 'roleNotAllowedHere', 'raw sim error');
      const m = mount();
      await settle();

      const simBtn = m.el.querySelector('.btn-simulate') as HTMLButtonElement | null;
      expect(simBtn).not.toBeNull();
      simBtn!.click();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw sim error');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toBe('raw sim error');
    });

    it('uses errorText in command failure toast (submit)', async () => {
      commandReplies['POST /api/rate-drafts/d1/submit'] = () => refusal(403, 'roleNotAllowedHere', 'raw sub error');
      const m = mount();
      await settle();

      const subBtn = m.el.querySelector('.btn-submit') as HTMLButtonElement | null;
      expect(subBtn).not.toBeNull();
      subBtn!.click();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw sub error');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toBe('raw sub error');
    });

    it('uses errorText in command failure toast (publish)', async () => {
      commandReplies['POST /api/rate-drafts/d1/publish'] = () => refusal(403, 'roleNotAllowedHere', 'raw pub error');
      const m = mount();
      await settle();

      const pubBtn = m.el.querySelector('.btn-publish') as HTMLButtonElement | null;
      expect(pubBtn).not.toBeNull();
      pubBtn!.click();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw pub error');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toBe('raw pub error');
    });

    it('uses errorText in command failure toast (adjustAll)', async () => {
      commandReplies['POST /api/rate-policies'] = () => refusal(403, 'roleNotAllowedHere', 'raw adj error');
      const m = mount();
      await settle();

      const adjBtn = m.el.querySelector('.btn-adjust-all') as HTMLButtonElement | null;
      expect(adjBtn).not.toBeNull();
      adjBtn!.click();
      await settle();

      const expected = errorText('roleNotAllowedHere', 'raw adj error');
      expect(toastText()).toBe(expected);
      expect(toastText()).not.toBe('raw adj error');
    });
  });

  // openHistory failure handling
  describe('openHistory failure handling', () => {
    it('shows worded toast when rate_publications fails with 500', async () => {
      publicationsReply = () => refusal(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      const m = mount();
      await settle();

      const buttons = Array.from(m.el.querySelectorAll('button'));
      const historyBtn = buttons.find((b) => b.textContent?.includes('ประวัติการปรับเรท'));
      expect(historyBtn, 'Rate history button exists').toBeDefined();
      historyBtn!.click();
      await settle();

      const expected = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
      expect(toastText()).toBe(expected);
      expect(document.querySelector('.rate-history-dialog')).toBeNull();
    });
  });
});
