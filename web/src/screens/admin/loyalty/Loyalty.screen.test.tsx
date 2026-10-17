// LY-6-FE (#2129): up-front screen tests, rows 1–3, the screen half of row 4, and rows 6–7.
// Written by the ticket's writer (claude/177aa7ef) before the screen exists. The implementer makes them pass and
// must not edit, delete or weaken them.
//
// Contract these tests read (all [required] in #2129 §7–8):
// - The screen reads `GET /api/v3/views/loyalty_rule_versions` (#2868, PR 2909): a bare `LoyaltyRuleVersionsData`
//   `{ pointVersions, tierVersions }`, newest `version` first (`views/loyalty_rule_versions.rs:216-217`).
// - Each version is one element with `data-version-id`, `data-kind` ("POINT" | "TIER"), `data-version` and
//   `data-status`, in the server's order. Only a DRAFT row offers `[data-action="submit"]`.
// - The POINT draft form is `[data-form="point-draft"]`; its controls are named after the keys PR 2050 forwards
//   (`rule_publisher.rs:10-20`): basis, pointsRatio, rounding, minimumBasisAmount, expiryMonths, validFrom, validTo.
//   Its button is `[data-action="save-draft"]`. `version` is not typed by the user: it is the highest POINT
//   `version` in the view + 1, because Loyalty keeps `UNIQUE (tenant_id, version)`.
// - Errors show in an element with `role="alert"` inside the screen.
//
// The menu is loaded through `import.meta.glob`, so a missing screen fails these tests on their result, not on an
// unresolved import. No threshold, ratio or tier name of the business appears here: the values are arbitrary test
// data, as the EXECUTIVE would type them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import type {
  LoyaltyRuleVersionsData,
  PointRuleVersionItem,
  TierRuleVersionItem,
} from '../../../shared/api/generated/loyalty_rule_versions';
import type { AdminMenu } from '../menus';

const found = import.meta.glob<{ menu: AdminMenu }>('./menu.ts', { eager: true });
const loyaltyMenu = (): AdminMenu | undefined => Object.values(found)[0]?.menu;

const VIEW = '/api/v3/views/loyalty_rule_versions';
const DRAFT = '/api/v3/commands/loyalty_rule_draft';
const SUBMIT = '/api/v3/commands/loyalty_rule_submit';

function point(versionId: string, version: number, status: string): PointRuleVersionItem {
  return {
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
  };
}

function tier(versionId: string, version: number, status: string): TierRuleVersionItem {
  return {
    versionId,
    version,
    status,
    rollingMonths: 3,
    tiers: [],
    grace: null,
    validFrom: '2026-10-01T00:00:00Z',
    validTo: null,
  };
}

interface Call {
  method: string;
  path: string;
  key: string | null;
  body: any;
}
interface Reply {
  status: number;
  body: unknown;
}

let versions: LoyaltyRuleVersionsData;
let viewFailure: Reply | null;
let draftReplies: Reply[];
let submitReplies: Reply[];
let calls: Call[];
let root: HTMLElement;
let dispose: (() => void) | undefined;

function json(r: Reply): Response {
  return new Response(JSON.stringify(r.body), {
    status: r.status,
    headers: { 'content-type': 'application/json' },
  });
}

function err(status: number, code: string, message: string): Reply {
  return { status, body: { error: { code, message } } };
}

beforeEach(() => {
  setLang('th');
  versions = {
    pointVersions: [point('pv-3', 3, 'DRAFT'), point('pv-2', 2, 'APPROVED'), point('pv-1', 1, 'SOMETHING_NEW')],
    tierVersions: [tier('tv-1', 1, 'APPROVED')],
  };
  viewFailure = null;
  draftReplies = [];
  submitReplies = [];
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
        return json(viewFailure ?? { status: 200, body: versions });
      }
      if (u.pathname === DRAFT && method === 'POST') {
        const r = draftReplies.shift() ?? err(500, 'serverError', 'no reply queued');
        if (r.status === 200) {
          const out = r.body as { versionId: string; status: string };
          versions = {
            ...versions,
            pointVersions: [point(out.versionId, body.fields.version, out.status), ...versions.pointVersions],
          };
        }
        return json(r);
      }
      if (u.pathname === SUBMIT && method === 'POST') {
        const r = submitReplies.shift() ?? err(500, 'serverError', 'no reply queued');
        if (r.status === 200) {
          const out = r.body as { versionId: string; status: string };
          versions = {
            ...versions,
            pointVersions: versions.pointVersions.map((v) =>
              v.versionId === out.versionId ? { ...v, status: out.status } : v,
            ),
          };
        }
        return json(r);
      }
      return json({ status: 200, body: { version: 1, changed: false } });
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

async function mount(): Promise<void> {
  const m = loyaltyMenu();
  expect(m, 'web/src/screens/admin/loyalty/menu.ts exports `menu`').toBeDefined();
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-01', to: '2026-10-17' });
  const [shown] = createSignal(true);
  const Component = m!.Component;
  dispose = render(() => <Component branch={branch} period={period} shown={shown} />, root);
  await settle();
}

function rows(kind?: 'POINT' | 'TIER'): HTMLElement[] {
  const all = [...root.querySelectorAll<HTMLElement>('[data-version-id]')];
  return kind ? all.filter((r) => r.dataset.kind === kind) : all;
}

function row(versionId: string): HTMLElement {
  const r = root.querySelector<HTMLElement>(`[data-version-id="${versionId}"]`);
  expect(r, `a row with data-version-id="${versionId}"`).not.toBeNull();
  return r!;
}

function alertText(): string {
  return [...root.querySelectorAll('[role="alert"]')].map((e) => e.textContent ?? '').join(' ');
}

function control(name: string): HTMLInputElement | HTMLSelectElement {
  const el = root.querySelector<HTMLInputElement | HTMLSelectElement>(`[data-form="point-draft"] [name="${name}"]`);
  expect(el, `the POINT draft form has a control named "${name}"`).not.toBeNull();
  return el!;
}

function set(name: string, value: string): void {
  const el = control(name);
  if (el instanceof HTMLSelectElement) {
    expect([...el.options].map((o) => o.value), `"${name}" offers ${value}`).toContain(value);
  }
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

function fillPointDraft(): void {
  set('basis', 'SETTLE_THB');
  set('pointsRatio', '0.5');
  set('rounding', 'HALF_UP');
  set('minimumBasisAmount', '100');
  set('expiryMonths', '12');
  setDay('validFrom', '2026-11-01');
}

function click(selector: string, within: ParentNode = root): void {
  const b = within.querySelector<HTMLButtonElement>(selector);
  expect(b, `a button ${selector}`).not.toBeNull();
  b!.click();
}

const draftCalls = () => calls.filter((c) => c.path === DRAFT);
const submitCalls = () => calls.filter((c) => c.path === SUBMIT);
const SENTINEL = 'raw server message';

describe('LY-6-FE row 1 (#2129 §9): the screen lists the current and draft versions from the view', () => {
  it('row 1: every POINT and TIER version is a row with its kind, version and status, POINT in the server order (newest first)', async () => {
    await mount();
    expect(calls.some((c) => c.method === 'GET' && c.path === VIEW), 'the screen reads the rule-version view').toBe(true);

    expect(rows('POINT').map((r) => r.dataset.versionId)).toEqual(['pv-3', 'pv-2', 'pv-1']);
    expect(rows('TIER').map((r) => r.dataset.versionId)).toEqual(['tv-1']);
    expect(row('pv-3').dataset.status).toBe('DRAFT');
    expect(row('pv-3').dataset.version).toBe('3');
    expect(row('pv-2').dataset.status).toBe('APPROVED');
    expect(row('tv-1').dataset.status).toBe('APPROVED');
  });

  it('row 1: only a DRAFT row offers Submit', async () => {
    await mount();
    expect(row('pv-3').querySelector('[data-action="submit"]'), 'the DRAFT row has Submit').not.toBeNull();
    expect(row('pv-2').querySelector('[data-action="submit"]'), 'an APPROVED row has no Submit').toBeNull();
    expect(row('tv-1').querySelector('[data-action="submit"]'), 'an APPROVED TIER row has no Submit').toBeNull();
  });
});

describe('LY-6-FE row 7 (G4): known statuses are worded, any other is shown as is', () => {
  it('row 7: APPROVED and DRAFT are worded (the raw code is not on screen); an unknown status is shown as given', async () => {
    await mount();
    expect(row('pv-2').textContent ?? '').not.toContain('APPROVED');
    expect(row('pv-3').textContent ?? '').not.toContain('DRAFT');
    expect(row('pv-1').textContent ?? '').toContain('SOMETHING_NEW');
  });
});

describe('LY-6-FE row 2 (#2129 §9): saving a POINT draft', () => {
  it('row 2: Save sends loyaltyRuleDraft with an Idempotency-Key, kind POINT, the typed fields, and version = highest POINT version + 1', async () => {
    draftReplies = [{ status: 200, body: { versionId: 'pv-4', status: 'DRAFT' } }];
    await mount();
    fillPointDraft();
    click('[data-form="point-draft"] [data-action="save-draft"]');
    await settle();

    expect(draftCalls()).toHaveLength(1);
    const c = draftCalls()[0]!;
    expect(c.key, 'an Idempotency-Key header').toBeTruthy();
    expect(c.body.kind).toBe('POINT');
    const f = c.body.fields;
    expect(f.version, 'version comes from the view: max(3, 2, 1) + 1').toBe(4);
    expect(typeof f.version).toBe('number');
    expect(f.basis).toBe('SETTLE_THB');
    expect(String(f.pointsRatio)).toBe('0.5');
    expect(f.rounding).toBe('HALF_UP');
    expect(String(f.minimumBasisAmount)).toBe('100');
    expect(f.expiryMonths, 'Loyalty reads expiryMonths as an integer').toBe(12);
    expect(typeof f.validFrom).toBe('string');
    expect(Number.isNaN(Date.parse(f.validFrom)), 'validFrom is a timestamp Loyalty can parse').toBe(false);
    expect(f.validTo == null, 'an empty optional field is left out or null, never ""').toBe(true);
  });

  it('row 2: after a 5xx, saving again with the same values reuses the same Idempotency-Key, and the new draft then shows', async () => {
    draftReplies = [err(500, 'serverError', SENTINEL), { status: 200, body: { versionId: 'pv-4', status: 'DRAFT' } }];
    await mount();
    fillPointDraft();
    click('[data-form="point-draft"] [data-action="save-draft"]');
    await settle();
    expect(alertText(), 'the failure is shown').not.toBe('');

    click('[data-form="point-draft"] [data-action="save-draft"]');
    await settle();
    expect(draftCalls()).toHaveLength(2);
    expect(draftCalls()[1]!.key, 'the retry reuses the first key').toBe(draftCalls()[0]!.key);
    expect(row('pv-4').dataset.status).toBe('DRAFT');
  });
});

describe('LY-6-FE row 3 (#2129 §9): submitting a draft', () => {
  it('row 3: Submit on the DRAFT row sends loyaltyRuleSubmit with its versionId and an Idempotency-Key, and the row then shows PENDING_APPROVAL', async () => {
    submitReplies = [{ status: 200, body: { versionId: 'pv-3', status: 'PENDING_APPROVAL', requestId: 'req-1' } }];
    await mount();
    click('[data-action="submit"]', row('pv-3'));
    await settle();

    expect(submitCalls()).toHaveLength(1);
    expect(submitCalls()[0]!.body).toEqual({ versionId: 'pv-3' });
    expect(submitCalls()[0]!.key, 'an Idempotency-Key header').toBeTruthy();
    expect(row('pv-3').dataset.status).toBe('PENDING_APPROVAL');
    expect(row('pv-3').querySelector('[data-action="submit"]'), 'a PENDING_APPROVAL row has no Submit').toBeNull();
  });
});

describe('LY-6-FE row 4 (screen) and row 6: 503 loyaltyUnavailable has its own words, not error.serverError', () => {
  it('row 4 (screen): a 503 loyaltyUnavailable on Save shows error.loyaltyUnavailable, not error.serverError', async () => {
    draftReplies = [err(503, 'loyaltyUnavailable', SENTINEL)];
    await mount();
    fillPointDraft();
    click('[data-form="point-draft"] [data-action="save-draft"]');
    await settle();

    const worded = errorText('loyaltyUnavailable', SENTINEL);
    expect(worded, 'error.loyaltyUnavailable exists (row 4, text half)').not.toBe(SENTINEL);
    expect(alertText()).toContain(worded);
    expect(alertText()).not.toContain(errorText('serverError', SENTINEL));
  });

  it('row 4 (screen): a 503 loyaltyUnavailable on Submit shows error.loyaltyUnavailable, not error.serverError', async () => {
    submitReplies = [err(503, 'loyaltyUnavailable', SENTINEL)];
    await mount();
    click('[data-action="submit"]', row('pv-3'));
    await settle();

    const worded = errorText('loyaltyUnavailable', SENTINEL);
    expect(worded).not.toBe(SENTINEL);
    expect(alertText()).toContain(worded);
    expect(alertText()).not.toContain(errorText('serverError', SENTINEL));
  });

  it('row 6: the view answering 503 loyaltyUnavailable shows error.loyaltyUnavailable, not error.serverError, and no version rows', async () => {
    viewFailure = err(503, 'loyaltyUnavailable', SENTINEL);
    await mount();

    const worded = errorText('loyaltyUnavailable', SENTINEL);
    expect(worded).not.toBe(SENTINEL);
    expect(alertText()).toContain(worded);
    expect(alertText()).not.toContain(errorText('serverError', SENTINEL));
    expect(rows()).toHaveLength(0);
  });
});
