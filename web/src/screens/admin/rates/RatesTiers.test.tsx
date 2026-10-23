// Written before the implementation by the test writer gemini/warut-3 (ticket author claude/e92db6), against VT-4 #1648's [proposed] DTO; re-checked when #1648 merges. Do not edit, delete or weaken.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal, type JSX } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { AdminMenu, AdminMenuProps } from '../menus';
import { Dialog, closeDialog } from '../parts/Dialog';
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

// ── [proposed] DTO literals from VT-4 #1648 ─────────────────────────────────

interface ValueTierLiteral {
  tierNo: number;
  upperInclusiveThb: string | null;
  buyMarginPct: string;
  sellMarginPct: string;
}

interface PolicyLiteral {
  id: string;
  version: number;
  branch: string;
  buyAdjustPct: string;
  sellAdjustPct: string;
  guards: Record<string, unknown>;
  maxReferenceAgeHours: number;
  onStale: string;
  staleLimitThb: string | null;
  reason?: string;
  valueTiersOn: boolean;
  valueTiers: ValueTierLiteral[];
  sharpMoveAlertPct: string | null;
}

function defaultPolicy(overrides: Partial<PolicyLiteral> = {}): PolicyLiteral {
  return {
    id: 'pol-1',
    version: 1,
    branch: 'MAIN',
    buyAdjustPct: '-0.5000',
    sellAdjustPct: '0.4000',
    guards: {},
    maxReferenceAgeHours: 24,
    onStale: 'STOP',
    staleLimitThb: null,
    reason: 'default policy',
    valueTiersOn: false,
    valueTiers: [],
    sharpMoveAlertPct: null,
    ...overrides,
  };
}

// ── network stub ─────────────────────────────────────────────────────────────

interface Sent {
  path: string;
  params: URLSearchParams;
  method: string;
  key: string | null;
  body?: Record<string, unknown>;
}

type Reply = { status: number; body: unknown };

let sent: Sent[];
let meRole: 'OWNER' | 'RATES';
let policy: PolicyLiteral;
let savePolicyReply: Reply;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
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

  if (s.path === '/api/v3/views/me') {
    return json(200, {
      version: 1,
      changed: true,
      data:
        meRole === 'OWNER'
          ? {
              kind: 'owner',
              id: 'owner-1',
              role: 'OWNER',
              branch: 'MAIN',
              name: 'Owner',
              username: 'owner',
              permissions: ['OverviewRead', 'RatesDraft', 'RatesHistoryRead', 'RatePolicySave', 'SystemSettings'],
            }
          : {
              kind: 'staff',
              id: 'employee-RATES',
              role: 'RATES',
              branch: 'MAIN',
              name: 'Rates',
              username: 'rates',
              permissions: ['RatesDraft', 'RatesHistoryRead'],
            },
    });
  }

  if (s.path === QUEUE_CARD_PATH) {
    return json(200, queueCardUnchanged());
  }

  if (s.method === 'GET') {
    if (s.path === '/api/v3/views/rate_policy') {
      return json(200, { version: policy.version, changed: true, data: { policy: structuredClone(policy) } });
    }
    if (s.path === '/api/v3/views/published_rates') {
      return json(200, {
        version: 1,
        changed: true,
        data: {
          publication: { id: 'pub-1', version: 1, publishedAt: '2026-10-14T02:00:00Z' },
          sourceDate: '2026-10-14',
          stale: false,
          lines: [
            { currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '32.00000000', sellRate: '33.00000000', lineId: 'line-USD' },
          ],
        },
      });
    }
    if (s.path === '/api/v3/views/rate_drafts') {
      return json(200, { version: 1, changed: true, data: { drafts: [] } });
    }
    if (s.path === '/api/v3/views/rate_draft') {
      return json(200, { version: 1, changed: true, data: { draft: null, lines: [] } });
    }
    if (s.path === '/api/v3/views/rates_reference') {
      return json(200, { version: 1, changed: true, data: { rows: [], fetchedAt: '2026-10-14T02:00:00Z', error: null } });
    }
    if (s.path === '/api/v3/views/rate_publications') {
      return json(200, { version: 1, changed: true, data: { publications: [] } });
    }
  }

  if (s.path === '/api/rate-policies' && s.method === 'POST') {
    return json(savePolicyReply.status, savePolicyReply.body);
  }

  return json(500, { error: { code: 'unexpected', message: `unhandled ${s.method} ${s.path}` } });
}

function policySaves(): Sent[] {
  return sent.filter((s) => s.path === '/api/rate-policies' && s.method === 'POST');
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
  const [branch, setBranch] = createSignal(init.branch ?? 'MAIN');
  const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
  const [shown] = createSignal(true);
  const Component = menu.Component;
  const el = document.createElement('div');
  root = el;
  document.body.appendChild(el);
  dispose = render(
    () => (
      <>
        <Component branch={branch} period={period} shown={shown} />
        <Dialog />
        <Toast />
      </>
    ),
    el,
  );
  return { el, setBranch };
}

// ── DOM helpers ──────────────────────────────────────────────────────────────

function textOf(el: Element | null | undefined): string {
  return (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
}

function pageText(el?: HTMLElement): string {
  const toastText = textOf(document.querySelector('.toast, [role="alert"]'));
  const containerText = textOf(el ?? document.body);
  return `${containerText} ${toastText}`;
}

function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

function toggle(input: HTMLInputElement, checked: boolean): void {
  input.checked = checked;
  input.dispatchEvent(new Event('click', { bubbles: true }));
  input.dispatchEvent(new Event('input', { bubbles: true }));
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

async function click(btn: HTMLButtonElement): Promise<void> {
  btn.click();
  await settle();
}

// ── DOM selector hooks for VT-5 ──────────────────────────────────────────────

function tiersSwitch(el: ParentNode): HTMLInputElement {
  const input = el.querySelector<HTMLInputElement>(
    'input[name="valueTiersOn"], input[data-field="valueTiersOn"], [data-testid="value-tiers-switch"], .value-tiers-switch input, input[type="checkbox"][aria-label*="tier" i], input[type="checkbox"][aria-label*="มูลค่า" i]',
  );
  expect(input, 'value tiers switch control').not.toBeNull();
  return input!;
}

function tierRows(el: ParentNode): HTMLElement[] {
  const rows = Array.from(
    el.querySelectorAll<HTMLElement>('[data-tier], [data-tier-row], tr[data-tier-no], .value-tier-row'),
  );
  expect(rows.length, 'four tier band rows').toBe(4);
  return rows;
}

function tierUpperInput(row: HTMLElement): HTMLInputElement | null {
  return row.querySelector<HTMLInputElement>(
    'input[name*="upper" i], [data-field="upperInclusiveThb"], [data-tier-field="upper"]',
  );
}

function tierBuyMarginInput(row: HTMLElement): HTMLInputElement {
  const input = row.querySelector<HTMLInputElement>(
    'input[name*="buyMargin" i], [data-field="buyMarginPct"], [data-tier-field="buy-margin"]',
  );
  expect(input, 'tier buy margin input').not.toBeNull();
  return input!;
}

function tierSellMarginInput(row: HTMLElement): HTMLInputElement {
  const input = row.querySelector<HTMLInputElement>(
    'input[name*="sellMargin" i], [data-field="sellMarginPct"], [data-tier-field="sell-margin"]',
  );
  expect(input, 'tier sell margin input').not.toBeNull();
  return input!;
}

function sharpMoveInput(el: ParentNode): HTMLInputElement {
  const input = el.querySelector<HTMLInputElement>(
    'input[name="sharpMoveAlertPct"], [data-field="sharpMoveAlertPct"], [data-testid="sharp-move-threshold"], input[aria-label*="sharp" i], input[aria-label*="ผันผวน" i]',
  );
  expect(input, 'sharp-move alert threshold input').not.toBeNull();
  return input!;
}

function savePolicyButton(el: ParentNode): HTMLButtonElement {
  const btn =
    el.querySelector<HTMLButtonElement>(
      'button[data-action="save-policy"], button.btn-save-policy, [data-testid="save-policy"], button.save-policy',
    ) ??
    Array.from(el.querySelectorAll<HTMLButtonElement>('button')).find((b) => {
      const txt = (b.textContent ?? '').trim();
      return (
        txt.includes('บันทึกนโยบาย') ||
        txt.includes('Save Policy') ||
        (txt.includes('บันทึก') && !txt.includes('ฉบับร่าง')) ||
        (txt.includes('Save') && !txt.includes('Draft'))
      );
    });
  expect(btn, 'save rate policy button').not.toBeNull();
  return btn!;
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe('VT-5 value tiers and sharp-move threshold (#1650)', () => {
  beforeEach(() => {
    sent = [];
    meRole = 'OWNER';
    policy = defaultPolicy();
    savePolicyReply = { status: 200, body: { id: 'pol-2', version: 2 } };
    vi.stubGlobal('fetch', vi.fn(fakeFetch));
    toast('');
    setLang('th');
  });

  afterEach(async () => {
    closeDialog();
    dispose?.();
    dispose = undefined;
    root?.remove();
    root = undefined;
    vi.unstubAllGlobals();
    setLang('th');
    await settle();
  });

  it('a view with valueTiersOn: false and no tiers renders the switch OFF and four empty rows (§9)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    expect(sw.checked).toBe(false);

    const rows = tierRows(el);
    expect(rows).toHaveLength(4);

    // Tiers 1-3 take an upper limit in THB and start empty
    for (let i = 0; i < 3; i++) {
      const upper = tierUpperInput(rows[i]!);
      expect(upper, `tier ${i + 1} upper limit input`).not.toBeNull();
      expect(upper!.value).toBe('');
    }

    // Tier 4 has no upper limit ("and above")
    const t4Upper = tierUpperInput(rows[3]!);
    expect(t4Upper === null || t4Upper.disabled || t4Upper.readOnly).toBe(true);

    // Margins per tier start empty
    for (let i = 0; i < 4; i++) {
      expect(tierBuyMarginInput(rows[i]!).value).toBe('');
      expect(tierSellMarginInput(rows[i]!).value).toBe('');
    }

    // Sharp-move alert % field starts empty
    expect(sharpMoveInput(el).value).toBe('');
  });

  it('amounts and percentages are kept and sent as strings exactly as typed (§7 [required], §9)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    toggle(sw, true);
    await settle();

    const rows = tierRows(el);
    type(tierUpperInput(rows[0]!)!, '20000.00');
    type(tierBuyMarginInput(rows[0]!), '0.0000');
    type(tierSellMarginInput(rows[0]!), '0.0000');

    type(tierUpperInput(rows[1]!)!, '100000.00');
    type(tierBuyMarginInput(rows[1]!), '+0.1000');
    type(tierSellMarginInput(rows[1]!), '-0.1000');

    type(tierUpperInput(rows[2]!)!, '500000.00');
    type(tierBuyMarginInput(rows[2]!), '+0.2000');
    type(tierSellMarginInput(rows[2]!), '-0.2000');

    type(tierBuyMarginInput(rows[3]!), '+0.3000');
    type(tierSellMarginInput(rows[3]!), '-0.3000');

    type(sharpMoveInput(el), '2.50');

    const saveBtn = savePolicyButton(el);
    await click(saveBtn);

    const saves = policySaves();
    expect(saves.length, 'policy save command sent').toBe(1);
    const body = saves[0]!.body as Record<string, unknown>;

    expect(body.valueTiersOn).toBe(true);
    expect(body.sharpMoveAlertPct).toBe('2.50');

    const tiers = body.valueTiers as Array<Record<string, unknown>>;
    expect(tiers).toHaveLength(4);
    expect(tiers[0]).toEqual({
      tierNo: 1,
      upperInclusiveThb: '20000.00',
      buyMarginPct: '0.0000',
      sellMarginPct: '0.0000',
    });
    expect(tiers[1]).toEqual({
      tierNo: 2,
      upperInclusiveThb: '100000.00',
      buyMarginPct: '+0.1000',
      sellMarginPct: '-0.1000',
    });
    expect(tiers[2]).toEqual({
      tierNo: 3,
      upperInclusiveThb: '500000.00',
      buyMarginPct: '+0.2000',
      sellMarginPct: '-0.2000',
    });
    expect(tiers[3]).toEqual({
      tierNo: 4,
      upperInclusiveThb: null,
      buyMarginPct: '+0.3000',
      sellMarginPct: '-0.3000',
    });
  });

  it('with the switch ON, non-rising bands block save and name the row (§7 [required], §9)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    toggle(sw, true);
    await settle();

    const rows = tierRows(el);
    // Non-rising bands: Tier 1 is 50000, Tier 2 is 20000 (not rising)
    type(tierUpperInput(rows[0]!)!, '50000.00');
    type(tierBuyMarginInput(rows[0]!), '0.0000');
    type(tierSellMarginInput(rows[0]!), '0.0000');

    type(tierUpperInput(rows[1]!)!, '20000.00');
    type(tierBuyMarginInput(rows[1]!), '0.1000');
    type(tierSellMarginInput(rows[1]!), '-0.1000');

    type(tierUpperInput(rows[2]!)!, '100000.00');
    type(tierBuyMarginInput(rows[2]!), '0.2000');
    type(tierSellMarginInput(rows[2]!), '-0.2000');

    type(tierBuyMarginInput(rows[3]!), '0.3000');
    type(tierSellMarginInput(rows[3]!), '-0.3000');

    const saveBtn = savePolicyButton(el);
    // §9: "ON with non-rising bands: Save is disabled, and the message names the row."
    expect(saveBtn.disabled, 'Save must be disabled when bands are non-rising (§9)').toBe(true);

    if (!saveBtn.disabled) {
      await click(saveBtn);
    }

    // Save must be refused in the screen: no POST /api/rate-policies sent
    expect(policySaves().length, 'save blocked on non-rising bands').toBe(0);

    // Validation must specifically name the offending row (Tier 2):
    // check via aria-invalid="true" on row 2's upper input, a row-scoped error element, or a dedicated alert/error element
    const row2Upper = tierUpperInput(rows[1]!)!;
    const row2Error = rows[1]!.querySelector<HTMLElement>(
      '[role="alert"], [data-error], .error, .text-error, .field-error, .invalid-feedback',
    );
    const alertError = el.querySelector<HTMLElement>(
      '[role="alert"], .alert-error, [data-field-error*="tier" i], .value-tiers-error',
    );

    const namesRowInMessage = (msg: string): boolean => {
      const lower = msg.toLowerCase();
      return (
        lower.includes('tier 2') ||
        lower.includes('ระดับ 2') ||
        lower.includes('แถวที่ 2') ||
        lower.includes('ขั้นที่ 2') ||
        (lower.includes('2') &&
          (lower.includes('tier') ||
            lower.includes('ระดับ') ||
            lower.includes('ขั้น') ||
            lower.includes('upper') ||
            lower.includes('band') ||
            lower.includes('น้อยกว่า') ||
            lower.includes('มากกว่า')))
      );
    };

    const hasAriaInvalid = row2Upper.getAttribute('aria-invalid') === 'true';
    const hasRowError = row2Error != null && namesRowInMessage(row2Error.textContent ?? '');
    const hasAlertError = alertError != null && namesRowInMessage(alertError.textContent ?? '');

    expect(
      hasAriaInvalid || hasRowError || hasAlertError,
      'validation names offending row 2 via aria-invalid="true", row-scoped error, or alert message',
    ).toBe(true);
  });

  it('turning the switch OFF keeps the band rows visible and sends them in the payload (§7 [required], §9)', async () => {
    const filledTiers: ValueTierLiteral[] = [
      { tierNo: 1, upperInclusiveThb: '20000.00', buyMarginPct: '0.0000', sellMarginPct: '0.0000' },
      { tierNo: 2, upperInclusiveThb: '100000.00', buyMarginPct: '0.1000', sellMarginPct: '-0.1000' },
      { tierNo: 3, upperInclusiveThb: '500000.00', buyMarginPct: '0.2000', sellMarginPct: '-0.2000' },
      { tierNo: 4, upperInclusiveThb: null, buyMarginPct: '0.3000', sellMarginPct: '-0.3000' },
    ];
    policy = defaultPolicy({
      valueTiersOn: true,
      valueTiers: filledTiers,
      sharpMoveAlertPct: '2.50',
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    expect(sw.checked).toBe(true);

    // Turning the switch OFF keeps the band rows visible
    toggle(sw, false);
    await settle();

    const rows = tierRows(el);
    expect(rows).toHaveLength(4);
    expect(tierUpperInput(rows[0]!)!.value).toBe('20000.00');

    // Save sends the rows unchanged with valueTiersOn: false
    const saveBtn = savePolicyButton(el);
    await click(saveBtn);

    const saves = policySaves();
    expect(saves.length).toBe(1);
    const body = saves[0]!.body as Record<string, unknown>;

    expect(body.valueTiersOn).toBe(false);
    expect(body.valueTiers).toEqual(filledTiers);
  });

  it('an empty sharp-move field sends null, and typed threshold is sent as string (§7 [required], §9)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: '3.00',
    });
    const { el } = mount();
    await settle();

    const thresh = sharpMoveInput(el);
    expect(thresh.value).toBe('3.00');

    // Clear threshold -> sends null
    type(thresh, '');
    const saveBtn = savePolicyButton(el);
    await click(saveBtn);

    let saves = policySaves();
    expect(saves.length).toBe(1);
    expect((saves[0]!.body as Record<string, unknown>).sharpMoveAlertPct).toBeNull();

    // Type threshold "2.5" -> sends string "2.5"
    type(thresh, '2.5');
    await click(saveBtn);

    saves = policySaves();
    expect(saves.length).toBe(2);
    expect((saves[1]!.body as Record<string, unknown>).sharpMoveAlertPct).toBe('2.5');
  });

  it('a non-owner sees the value tiers section read-only (§7 [required], §9)', async () => {
    meRole = 'RATES'; // non-owner role without frame change permission
    policy = defaultPolicy({
      valueTiersOn: true,
      valueTiers: [
        { tierNo: 1, upperInclusiveThb: '20000.00', buyMarginPct: '0.0000', sellMarginPct: '0.0000' },
        { tierNo: 2, upperInclusiveThb: '100000.00', buyMarginPct: '0.1000', sellMarginPct: '-0.1000' },
        { tierNo: 3, upperInclusiveThb: '500000.00', buyMarginPct: '0.2000', sellMarginPct: '-0.2000' },
        { tierNo: 4, upperInclusiveThb: null, buyMarginPct: '0.3000', sellMarginPct: '-0.3000' },
      ],
      sharpMoveAlertPct: '2.50',
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    expect(sw.disabled, 'value tiers switch is disabled for non-owner').toBe(true);

    const rows = tierRows(el);
    for (let i = 0; i < 3; i++) {
      const upper = tierUpperInput(rows[i]!)!;
      expect(upper.disabled || upper.readOnly, `tier ${i + 1} upper is read-only`).toBe(true);
    }
    for (let i = 0; i < 4; i++) {
      const buy = tierBuyMarginInput(rows[i]!);
      const sell = tierSellMarginInput(rows[i]!);
      expect(buy.disabled || buy.readOnly, `tier ${i + 1} buy margin is read-only`).toBe(true);
      expect(sell.disabled || sell.readOnly, `tier ${i + 1} sell margin is read-only`).toBe(true);
    }

    const thresh = sharpMoveInput(el);
    expect(thresh.disabled || thresh.readOnly, 'sharp-move threshold is read-only').toBe(true);

    const saveBtn = savePolicyButton(el);
    expect(saveBtn.disabled, 'save button is disabled for non-owner').toBe(true);
    if (!saveBtn.disabled) {
      await click(saveBtn);
    }
    expect(policySaves().length, 'no policy save sent by non-owner').toBe(0);
  });

  it('one Idempotency-Key is kept across save retries (§8)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
    const { el } = mount();
    await settle();

    // Server fails on first attempt
    savePolicyReply = { status: 500, body: { error: { code: 'serverError', message: 'internal error' } } };

    const sw = tiersSwitch(el);
    toggle(sw, true);
    await settle();

    const rows = tierRows(el);
    type(tierUpperInput(rows[0]!)!, '20000.00');
    type(tierBuyMarginInput(rows[0]!), '0.0000');
    type(tierSellMarginInput(rows[0]!), '0.0000');
    type(tierUpperInput(rows[1]!)!, '100000.00');
    type(tierBuyMarginInput(rows[1]!), '0.1000');
    type(tierSellMarginInput(rows[1]!), '-0.1000');
    type(tierUpperInput(rows[2]!)!, '500000.00');
    type(tierBuyMarginInput(rows[2]!), '0.2000');
    type(tierSellMarginInput(rows[2]!), '-0.2000');
    type(tierBuyMarginInput(rows[3]!), '0.3000');
    type(tierSellMarginInput(rows[3]!), '-0.3000');

    const saveBtn = savePolicyButton(el);
    await click(saveBtn);

    let saves = policySaves();
    expect(saves.length).toBe(1);
    const key1 = saves[0]!.key;
    expect(key1, 'first save attempt carries idempotency key').toBeTruthy();

    // Server recovers on retry
    savePolicyReply = { status: 200, body: { id: 'pol-2', version: 2 } };

    // User retries clicking save without editing form
    await click(saveBtn);

    saves = policySaves();
    expect(saves.length).toBe(2);
    const key2 = saves[1]!.key;
    expect(key2, 'retry attempt reuses the exact same idempotency key').toBe(key1);
  });

  it('server errors are shown worded on refusal (§7 [required], §8)', async () => {
    policy = defaultPolicy({
      valueTiersOn: false,
      valueTiers: [],
      sharpMoveAlertPct: null,
    });
    const { el } = mount();
    await settle();

    const sw = tiersSwitch(el);
    toggle(sw, true);
    await settle();

    const rows = tierRows(el);
    type(tierUpperInput(rows[0]!)!, '20000.00');
    type(tierBuyMarginInput(rows[0]!), '0.0000');
    type(tierSellMarginInput(rows[0]!), '0.0000');
    type(tierUpperInput(rows[1]!)!, '100000.00');
    type(tierBuyMarginInput(rows[1]!), '0.1000');
    type(tierSellMarginInput(rows[1]!), '-0.1000');
    type(tierUpperInput(rows[2]!)!, '500000.00');
    type(tierBuyMarginInput(rows[2]!), '0.2000');
    type(tierSellMarginInput(rows[2]!), '-0.2000');
    type(tierBuyMarginInput(rows[3]!), '0.3000');
    type(tierSellMarginInput(rows[3]!), '-0.3000');

    // Server returns validationFailed
    savePolicyReply = {
      status: 400,
      body: { error: { code: 'validationFailed', message: 'margin exceeds tier guard' } },
    };

    const saveBtn = savePolicyButton(el);
    await click(saveBtn);

    expect(pageText(el), 'worded validationFailed message is shown').toContain('ข้อมูลไม่ถูกต้อง กรุณาตรวจสอบค่าที่กรอก');

    // Server returns staleVersion
    savePolicyReply = {
      status: 409,
      body: { error: { code: 'staleVersion', message: 'policy version changed' } },
    };

    await click(saveBtn);
    expect(pageText(el), 'worded staleVersion message is shown').toContain('ข้อมูลมีการเปลี่ยนแปลง กรุณาโหลดหน้าใหม่แล้วลองอีกครั้ง');
  });
});
