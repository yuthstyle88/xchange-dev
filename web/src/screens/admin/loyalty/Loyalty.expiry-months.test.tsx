// T17 (#4917): Admin loyalty expiry-months validation (L1–L4).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import {
  makeLoyaltyRuleVersionsData,
  makePointRuleVersionItem,
  type LoyaltyRuleVersionsData,
} from '../../../shared/testing/fixtures';
import type { AdminMenu } from '../menus';
import { th } from './i18n';

const found = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const loyaltyMenu = (): AdminMenu | undefined => Object.values(found)[0]?.menu;

const VIEW = '/api/v3/views/loyalty_rule_versions';
const DRAFT = '/api/v3/commands/loyalty_rule_draft';

function point(versionId: string, version: number, status: string) {
  return makePointRuleVersionItem({
    versionId,
    version,
    status,
    basis: 'THB_AMOUNT',
    pointsRatio: '1.25',
    rounding: 'FLOOR',
    minimumBasisAmount: '0',
    expiryMonths: 6,
    validFrom: '2026-10-01T00:00:00Z',
    validTo: null,
  });
}

interface Call {
  method: string;
  path: string;
  key: string | null;
  body: any;
}

let versions: LoyaltyRuleVersionsData;
let calls: Call[];
let root: HTMLElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  versions = makeLoyaltyRuleVersionsData({
    pointVersions: [point('pv-3', 3, 'DRAFT')],
    tierVersions: [],
  });
  calls = [];
  root = document.createElement('div');
  document.body.appendChild(root);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit) => {
      const u = new URL(String(input), 'http://desk.test');
      const method = (init?.method ?? 'GET').toUpperCase();
      const key = new Headers(init?.headers ?? {}).get('idempotency-key');
      const body = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
      calls.push({ method, path: u.pathname, key, body });
      if (u.pathname === VIEW) {
        return new Response(JSON.stringify(versions), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      if (u.pathname === DRAFT && method === 'POST') {
        return new Response(JSON.stringify({ versionId: 'pv-4', status: 'DRAFT' }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({ version: 1, changed: false }), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }),
  );
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  root?.remove();
  vi.unstubAllGlobals();
  setLang('th');
});

async function settle(): Promise<void> {
  for (let i = 0; i < 8; i++) await new Promise((r) => setTimeout(r, 0));
}

function control(name: string): HTMLInputElement | HTMLSelectElement {
  const el = root.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-form="point-draft"] [name="${name}"]`);
  expect(el, `the POINT draft form has a control named "${name}"`).not.toBeNull();
  return el!;
}

function set(name: string, value: string): void {
  const el = control(name);
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function setDay(name: string, day: string): void {
  const el = control(name) as HTMLInputElement;
  const v = el.type === 'date' ? day : el.type === 'datetime-local' ? `${day}T00:00` : `${day}T00:00:00+07:00`;
  el.value = v;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

async function mountAndFillBase(): Promise<void> {
  const m = loyaltyMenu();
  expect(m).toBeDefined();
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-17' });
  const [shown] = createSignal(true);
  const Component = m!.Component;
  dispose = render(() => <Component branch={branch} period={period} shown={shown} />, root);
  await settle();

  set('basis', 'SETTLE_THB');
  set('pointsRatio', '0.5');
  set('rounding', 'HALF_UP');
  set('minimumBasisAmount', '100');
  setDay('validFrom', '2026-11-01');
}

function clickSave(): void {
  const saveBtn = root.querySelector<HTMLButtonElement>('[data-form="point-draft"] [data-action="save-draft"]');
  expect(saveBtn).not.toBeNull();
  saveBtn!.click();
}

function expiryFieldAlert(): HTMLElement | null {
  const group = control('expiryMonths').closest('.xc-form-group');
  return group?.querySelector<HTMLElement>('[role="alert"]') ?? null;
}

const draftCalls = () => calls.filter((c) => c.path === DRAFT);

describe('T17 (#4917): Admin loyalty expiryMonths validation', () => {
  it('L1: expiryMonths "1.5" shows a worded line at the field and sends no request', async () => {
    await mountAndFillBase();
    set('expiryMonths', '1.5');
    clickSave();
    await settle();

    expect(draftCalls()).toHaveLength(0);
    const alert = expiryFieldAlert();
    expect(alert, 'worded line shown at expiryMonths field').not.toBeNull();
    expect(alert!.textContent?.trim()).toBe(th.expiryMonthsInvalid);
  });

  it('L2: expiryMonths "abc" shows a worded line at the field and sends no request', async () => {
    await mountAndFillBase();
    set('expiryMonths', 'abc');
    clickSave();
    await settle();

    expect(draftCalls()).toHaveLength(0);
    const alert = expiryFieldAlert();
    expect(alert, 'worded line shown at expiryMonths field').not.toBeNull();
    expect(alert!.textContent?.trim()).toBe(th.expiryMonthsInvalid);
  });

  it('L3: empty expiryMonths is sent as null (as today)', async () => {
    await mountAndFillBase();
    set('expiryMonths', '');
    clickSave();
    await settle();

    expect(expiryFieldAlert()).toBeNull();
    expect(draftCalls()).toHaveLength(1);
    expect(draftCalls()[0]!.body.fields.expiryMonths).toBeNull();
  });

  it('L4: expiryMonths "12" is sent as 12', async () => {
    await mountAndFillBase();
    set('expiryMonths', '12');
    clickSave();
    await settle();

    expect(expiryFieldAlert()).toBeNull();
    expect(draftCalls()).toHaveLength(1);
    expect(draftCalls()[0]!.body.fields.expiryMonths).toBe(12);
  });
});
