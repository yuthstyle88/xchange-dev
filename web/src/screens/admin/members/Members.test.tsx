// Written before the implementation by the test writer gemini/warut-6 (ticket author claude/e92db6). Do not edit, delete or weaken.
// Amended by gemini/warut-2 per reviewer claude/dell-claude-3 (#1407 issuecomment-6158533621).
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../../../shared/api/client';
import { setLang } from '../../../shared/i18n';
import { meOf, permissionsOf, rolesOf } from '../../../test-support/me';
import { AdminShell } from '../AdminShell';
import type { AdminMenuProps } from '../menus';
import { menus } from '../menus';
import * as table from './i18n';
import { menu } from './menu';
import { Members } from './Members';

interface SentRequest {
  method: string;
  url: string;
  pathname: string;
  search: string;
  headers: Record<string, string>;
  body: unknown;
}

function dummyProps(overrides: Partial<AdminMenuProps> = {}): AdminMenuProps {
  const [branch] = createSignal('MAIN');
  const [period] = createSignal({ from: '2026-10-14', to: '2026-10-14' });
  const [shown] = createSignal(true);
  return {
    branch,
    period,
    shown,
    ...overrides,
  };
}

const realSetTimeout = globalThis.setTimeout;
async function settle(): Promise<void> {
  for (let i = 0; i < 12; i++) {
    if (globalThis.setTimeout !== realSetTimeout) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => realSetTimeout(r, 0));
  }
}

describe('MK-8 M03: Members screen and segments (#1407)', () => {
  let root: HTMLElement;
  let dispose: (() => void) | undefined;
  let sentRequests: SentRequest[] = [];

  let meFixture = meOf({
    id: 'u-mkt',
    role: 'MARKETING',
    branch: 'MAIN',
    name: 'Marketing Person',
    branchName: 'สำนักงานใหญ่',
    branches: [{ id: 'MAIN', code: 'MAIN', name: 'สำนักงานใหญ่' }],
  });

  // Mock server state
  let defineStatus = 200;
  let defineResponse: unknown = { id: '00000000-0000-0000-0000-000000000001', version: 1 };
  let defineFailFirst = false;

  let snapshotStatus = 200;
  let snapshotResponse: unknown = { id: '00000000-0000-0000-0000-000000000002', memberCount: 42 };

  let previewStatus = 200;
  let previewResponse: unknown = {
    snapshotId: '00000000-0000-0000-0000-000000000002',
    memberCount: 120,
    distribution: {
      avgRecencyDays: 14,
      avgFrequency: 3.5,
      totalValueThb: '150000.00',
    },
    sample: [
      {
        customerId: 'cust-01',
        maskedName: 'Som**** Prasert',
        // Stray fields to guard against spreading the raw row (#1407 item 8):
        displayName: 'Somsak Prasert',
        phone: '0819876543',
        passport: 'AB1234567',
        metrics: {
          recencyDays: 10,
          frequency: 2,
          valueThb: '5000.00',
          tierCode: 'GOLD',
        },
      },
      {
        customerId: 'cust-02',
        maskedName: 'Wic**** Thongchai',
        displayName: 'Wichai Thongchai',
        phone: '0891234567',
        passport: 'CD7654321',
        metrics: {
          recencyDays: 25,
          frequency: 4,
          valueThb: '12000.00',
          tierCode: 'SILVER',
        },
      },
    ],
  };

  beforeEach(() => {
    setLang('th');
    root = document.createElement('div');
    document.body.appendChild(root);
    sentRequests = [];
    defineStatus = 200;
    defineResponse = { id: '00000000-0000-0000-0000-000000000001', version: 1 };
    defineFailFirst = false;
    snapshotStatus = 200;
    snapshotResponse = { id: '00000000-0000-0000-0000-000000000002', memberCount: 42 };
    previewStatus = 200;

    vi.stubGlobal('matchMedia', (query: string) => ({
      media: query,
      matches: false,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
      onchange: null,
      dispatchEvent: () => false,
    }));

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(raw, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      const headers: Record<string, string> = {};
      if (init?.headers) {
        new Headers(init.headers).forEach((v, k) => {
          headers[k.toLowerCase()] = v;
        });
      }
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({ method, url: raw, pathname: u.pathname, search: u.search, headers, body });

      if (u.pathname === '/api/v3/views/me') {
        return new Response(JSON.stringify({ version: 1, changed: true, data: meFixture }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      if (u.pathname === '/api/v3/views/admin_branches') {
        return new Response(
          JSON.stringify({ version: 1, changed: true, data: { branches: [{ id: 'MAIN', name: 'สำนักงานใหญ่' }] } }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }

      if (u.pathname === '/api/v3/views/kiosk_notifications') {
        return new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Exact path only: POST /api/segments (#1407 item 3)
      if (method === 'POST' && u.pathname === '/api/segments') {
        if (defineFailFirst) {
          defineFailFirst = false;
          throw new TypeError('NetworkError when attempting to fetch resource.');
        }
        return new Response(JSON.stringify(defineResponse), {
          status: defineStatus,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Exact path only: POST /api/segments/<id>/snapshots (#1407 item 3)
      if (method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(u.pathname)) {
        return new Response(JSON.stringify(snapshotResponse), {
          status: snapshotStatus,
          headers: { 'content-type': 'application/json' },
        });
      }

      // Exact path only: GET /api/v3/views/segment_preview (#1407 item 3, 4: envelope only)
      if (method === 'GET' && u.pathname === '/api/v3/views/segment_preview') {
        return new Response(
          JSON.stringify({
            version: 1,
            changed: true,
            data: previewResponse,
          }),
          { status: previewStatus, headers: { 'content-type': 'application/json' } },
        );
      }

      return new Response(JSON.stringify({ error: { code: 'notFound', message: 'Not found' } }), {
        status: 404,
        headers: { 'content-type': 'application/json' },
      });
    });
  });

  afterEach(async () => {
    await settle();
    dispose?.();
    dispose = undefined;
    root?.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals(); // #1407 item 10: un-stub globals along with restoreAllMocks
    vi.restoreAllMocks();
    location.hash = '';
  });

  function mountComponent(props: Partial<AdminMenuProps> = {}) {
    dispose = render(() => <Members {...dummyProps(props)} />, root);
  }

  async function driveDefine(name = 'High Value VIP', recencyDays = 30) {
    const nameInput = root.querySelector('input[name="name"], input[data-field="name"], #segment-name') as HTMLInputElement;
    expect(nameInput, 'segment name input rendered').not.toBeNull();
    nameInput.value = name;
    nameInput.dispatchEvent(new Event('input', { bubbles: true }));

    const recencyInput = root.querySelector(
      'input[name="recency_days"], input[data-field="recency_days"], #segment-recency-days',
    ) as HTMLInputElement;
    if (recencyInput) {
      recencyInput.value = String(recencyDays);
      recencyInput.dispatchEvent(new Event('input', { bubbles: true }));
    }

    const submitBtn = root.querySelector(
      'button[type="submit"], button[data-action="define"], button[data-action="save"], .xc-members-define-btn',
    ) as HTMLButtonElement;
    expect(submitBtn, 'define / submit button rendered').not.toBeNull();
    submitBtn.click();
    await settle();
  }

  async function driveSnapshot() {
    const snapshotBtn = root.querySelector(
      'button[data-action="snapshot"], button[data-action="take-snapshot"], .xc-members-snapshot-btn',
    ) as HTMLButtonElement;
    expect(snapshotBtn, 'snapshot button rendered').not.toBeNull();
    snapshotBtn.click();
    await settle();
  }

  // ── 1. Permission and Menu Visibility ──────────────────────────────────────────

  describe('1. Menu and permission gating: MARKETING sees the menu, TELLER does not', () => {
    it('pins the menu record: id members, needs MarketingMembers, order 26, group data', () => {
      expect(menu, 'menu is defined').toBeDefined();
      expect(menu?.id).toBe('members');
      expect(menu?.needs).toBe('MarketingMembers');
      expect(menu?.order).toBe(26); // 23–25 are campaigns, campaign-performance and booking-slots on main (rebase onto 40303f11)
      expect(menu?.group).toBe('data');
      expect(menu?.dated).toBe(false);
      expect(typeof menu?.label()).toBe('string');
      expect(typeof menu?.heading?.title()).toBe('string');
      expect(typeof menu?.Component).toBe('function');
    });

    it('MARKETING role holds MarketingMembers, while TELLER does not', () => {
      const marketingPerms = permissionsOf(rolesOf('MARKETING', 'MAIN'));
      expect(marketingPerms).toContain('MarketingMembers');

      const tellerPerms = permissionsOf(rolesOf('TELLER', 'MAIN'));
      expect(tellerPerms).not.toContain('MarketingMembers');

      for (const role of ['TELLER', 'BRANCH_MANAGER', 'FINANCE', 'VERIFICATION_REVIEWER']) {
        expect(permissionsOf(rolesOf(role, 'MAIN'))).not.toContain('MarketingMembers');
      }
    });

    it('in AdminShell: MARKETING role sees the members menu link, while TELLER does not', async () => {
      // MARKETING sees members menu link
      meFixture = meOf({ id: 'u-mkt', role: 'MARKETING', branch: 'MAIN', name: 'Marketing', branchName: 'MAIN', branches: [] });
      dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
      await settle();
      await settle();

      expect(root.querySelector('a[href="#members"]'), 'MARKETING sees #members menu link').not.toBeNull();

      // Clean up first render
      dispose();
      dispose = undefined;
      root.innerHTML = '';

      // TELLER does not see members menu link
      meFixture = meOf({ id: 'u-tel', role: 'TELLER', branch: 'MAIN', name: 'Teller', branchName: 'MAIN', branches: [] });
      dispose = render(() => <AdminShell api={new ApiClient()} menus={menus} />, root);
      await settle();
      await settle();

      expect(root.querySelector('a[href="#members"]'), 'TELLER does not see #members menu link').toBeNull();
    });
  });

  // ── 2. Define a Segment ────────────────────────────────────────────────────────

  describe('2. Define a segment (segmentDefine with one Idempotency-Key kept across retries)', () => {
    it('submits segmentDefine command to POST /api/v3/commands/segment_define with Idempotency-Key header', async () => {
      mountComponent();
      await settle();

      await driveDefine('High Value VIP', 30);

      const defineReq = sentRequests.find((r) => r.method === 'POST' && r.pathname === '/api/segments');
      expect(defineReq, 'sends segmentDefine request').toBeDefined();
      expect(defineReq!.headers['idempotency-key'], 'has non-empty Idempotency-Key header').toBeTruthy();
      // #1407 item 6: pins name and rule with snake_case keys (no rename_all in SegmentRule)
      expect(defineReq!.body).toMatchObject({
        name: 'High Value VIP',
        rule: {
          recency_days: 30,
        },
      });
    });

    it('keeps the same Idempotency-Key across retries after a network failure', async () => {
      defineFailFirst = true; // First request throws NetworkError
      mountComponent();
      await settle();

      const nameInput = root.querySelector('input[name="name"], input[data-field="name"], #segment-name') as HTMLInputElement;
      expect(nameInput).not.toBeNull();
      nameInput.value = 'Active Spenders';
      nameInput.dispatchEvent(new Event('input', { bubbles: true }));

      const submitBtn = root.querySelector(
        'button[type="submit"], button[data-action="define"], button[data-action="save"], .xc-members-define-btn',
      ) as HTMLButtonElement;
      expect(submitBtn).not.toBeNull();

      // First click - fails with network error
      submitBtn.click();
      await settle();

      // Second click - retries
      submitBtn.click();
      await settle();

      const defineReqs = sentRequests.filter(
        (r) => r.method === 'POST' && r.pathname === '/api/segments',
      );
      expect(defineReqs.length, 'two attempts made').toBe(2);
      expect(defineReqs[0].headers['idempotency-key']).toBeTruthy();
      expect(defineReqs[1].headers['idempotency-key'], 'same Idempotency-Key is reused on retry').toBe(
        defineReqs[0].headers['idempotency-key'],
      );
    });
  });

  // ── 3. Take a Snapshot ────────────────────────────────────────────────────────

  describe('3. Take a snapshot (segmentSnapshot with definition id) and show its count', () => {
    it('sends segmentSnapshot command with definition id and Idempotency-Key, updates DOM with memberCount', async () => {
      defineResponse = { id: '00000000-0000-0000-0000-000000000001', version: 1 };
      snapshotResponse = { id: '00000000-0000-0000-0000-000000000002', memberCount: 42 };

      mountComponent();
      await settle();

      // 1. Define first to produce definition id
      await driveDefine('High Value VIP', 30);

      // 2. Take snapshot of this definition
      await driveSnapshot();

      const snapReq = sentRequests.find((r) => r.method === 'POST' && /^\/api\/segments\/[^/]+\/snapshots$/.test(r.pathname));
      expect(snapReq, 'sends segmentSnapshot request').toBeDefined();
      expect(snapReq!.headers['idempotency-key'], 'has non-empty Idempotency-Key header').toBeTruthy();
      // #1407 item 5: snapshot takes { id }, the definition id from define output
      expect(snapReq!.body).toEqual({
        id: '00000000-0000-0000-0000-000000000001',
      });

      await vi.waitFor(() => {
        expect(root.textContent).toContain('42');
      });
    });
  });

  // ── 4. Preview Segment and Privacy Guard ───────────────────────────────────────

  describe('4. Preview shows count, metric distribution and MASKED names only (real flow: define -> snapshot -> preview)', () => {
    it('drives the real flow and requests preview with snapshot id, showing count and distribution in MoneyText format', async () => {
      mountComponent();
      await settle();

      // Drive real flow: define -> snapshot -> preview (#1407 item 5)
      await driveDefine('High Value VIP', 30);
      await driveSnapshot();

      const prevReq = sentRequests.find((r) => r.method === 'GET' && r.pathname === '/api/v3/views/segment_preview');
      expect(prevReq, 'requests segment_preview view').toBeDefined();
      // #1407 item 5: segment_preview takes ?id=<snapshot_id>
      expect(prevReq!.search, 'passes snapshot id in query').toContain('id=00000000-0000-0000-0000-000000000002');

      await vi.waitFor(() => {
        const text = root.textContent ?? '';
        expect(text).toContain('120'); // member count
        expect(text).toContain('14'); // avgRecencyDays
        expect(text).toContain('3.5'); // avgFrequency
        // #1407 item 9: money format grouped with comma (150,000.00)
        expect(text).toContain('150,000.00');
      });
    });

    it('renders MASKED names only, and NEVER exposes unmasked names, customerId as name, or stray PII in the DOM', async () => {
      mountComponent();
      await settle();

      await driveDefine('High Value VIP', 30);
      await driveSnapshot();

      await vi.waitFor(() => {
        const text = root.textContent ?? '';
        // Masked names are rendered
        expect(text).toContain('Som**** Prasert');
        expect(text).toContain('Wic**** Thongchai');

        // #1407 item 8: Unmasked names from stray fields in mock must NOT be in the DOM
        expect(text).not.toContain('Somsak Prasert');
        expect(text).not.toContain('Wichai Thongchai');

        // Stray identity values (phones, passports) must NEVER appear in the DOM
        expect(text).not.toContain('0819876543');
        expect(text).not.toContain('0891234567');
        expect(text).not.toContain('AB1234567');
        expect(text).not.toContain('CD7654321');

        // customerId must not be shown as a display name
        expect(text).not.toContain('cust-01');
        expect(text).not.toContain('cust-02');
      });
    });
  });

  // ── 5. Worded Errors ──────────────────────────────────────────────────────────

  describe('5. Worded errors (validationFailed, outOfScope, roleNotAllowed - all 400)', () => {
    it('validationFailed (400): shows worded validation error from i18n and absent raw code', async () => {
      defineStatus = 400;
      defineResponse = {
        error: {
          code: 'validationFailed',
          field: 'name',
          message: 'Validation failed',
        },
      };

      mountComponent();
      await driveDefine('High Value VIP', 30);

      await vi.waitFor(() => {
        const text = root.textContent ?? '';
        // #1407 item 7: pinned i18n text, raw code must be absent
        expect(text).toContain(table.th['error.validationFailed']);
        expect(text).not.toContain('validationFailed');
      });
    });

    it('outOfScope (400): shows worded out-of-scope error from i18n and absent raw code', async () => {
      // #1407 item 7: outOfScope is 400, not 403 (MK-6 contract)
      defineStatus = 400;
      defineResponse = {
        error: {
          code: 'outOfScope',
          message: 'Branch out of scope',
        },
      };

      mountComponent();
      await settle();

      await driveDefine('High Value VIP', 30);

      await vi.waitFor(() => {
        const text = root.textContent ?? '';
        expect(text).toContain(table.th['error.outOfScope']);
        expect(text).not.toContain('outOfScope');
      });
    });

    it('roleNotAllowed (400): shows worded permission refusal from i18n and absent raw code', async () => {
      // #1407 item 7: permission refusal roleNotAllowed (400)
      defineStatus = 400;
      defineResponse = {
        error: {
          code: 'roleNotAllowed',
          message: 'Role not allowed',
        },
      };

      mountComponent();
      await settle();

      await driveDefine('High Value VIP', 30);

      await vi.waitFor(() => {
        const text = root.textContent ?? '';
        expect(text).toContain(table.th['error.roleNotAllowed']);
        expect(text).not.toContain('roleNotAllowed');
      });
    });
  });
});
