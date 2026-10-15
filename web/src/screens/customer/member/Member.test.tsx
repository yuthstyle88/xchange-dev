// Written before the implementation by the test writer gemini/warut-2 (ticket author claude/e92db6). Do not edit, delete or weaken.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { MemberScreen } from './Member';
import type { MyMembershipData } from '../../../shared/api/generated/my_membership';
import * as table from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: string): string;
  existsSync(file: string): boolean;
}

interface NodePath {
  resolve(...segments: string[]): string;
  join(...segments: string[]): string;
  dirname(p: string): string;
}

const fsName = 'node:fs';
const pathName = 'node:path';
let fs: NodeFs;
let path: NodePath;
let thisDir: string;

interface SentRequest {
  method: string;
  url: string;
  path: string;
  search: string;
  body: unknown;
}

// Raw JSON literals — NO generated type is imported (LY-5 is not in main; the implementer
// will replace raw mock handling with the generated client call when LY-5 merges).
const MOCK_MEMBER_FULL = {
  tier: 'GOLD',
  effectiveUntil: '2026-11-15T00:00:00Z',
  points: {
    available: 100,
    pending: 25,
  },
  pointsUnavailable: false,
  nextExpiry: {
    points: 50,
    at: '2026-12-31T23:59:59Z',
  },
  progress: {
    basis: 'VOLUME_THB',
    current: '12500.00',
    next: '20000.00',
  },
} satisfies MyMembershipData;

const MOCK_LOYALTY_DOWN = {
  tier: 'PLATINUM',
  effectiveUntil: '2026-11-15T00:00:00Z',
  points: null,
  pointsUnavailable: true,
  nextExpiry: null,
  progress: null,
} satisfies MyMembershipData;

const MOCK_NOT_MEMBER = {
  tier: null,
  effectiveUntil: null,
  points: {
    available: 0,
    pending: 0,
  },
  pointsUnavailable: false,
  nextExpiry: null,
  progress: null,
} satisfies MyMembershipData;

const MOCK_TOP_TIER = {
  tier: 'DIAMOND',
  effectiveUntil: '2027-01-01T00:00:00Z',
  points: {
    available: 5000,
    pending: 0,
  },
  pointsUnavailable: false,
  nextExpiry: null,
  progress: {
    basis: 'VOLUME_THB',
    current: '100000.00',
    next: null,
  },
} satisfies MyMembershipData;

const MOCK_PENDING_SEPARATE = {
  tier: 'SILVER',
  effectiveUntil: '2026-12-31T00:00:00Z',
  points: {
    available: 500,
    pending: 120,
  },
  pointsUnavailable: false,
  nextExpiry: null,
  progress: null,
} satisfies MyMembershipData;

describe('APP-19 / C05: Customer Membership Screen MemberScreen (#1649)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let sentRequests: SentRequest[] = [];
  let viewResponse: unknown = MOCK_MEMBER_FULL;
  let viewStatus = 200;
  let viewDelayGate: Promise<void> | null = null;

  beforeAll(async () => {
    fs = (await import(/* @vite-ignore */ fsName)) as unknown as NodeFs;
    path = (await import(/* @vite-ignore */ pathName)) as unknown as NodePath;
    const thisFile = new URL(import.meta.url).pathname;
    thisDir = path.dirname(thisFile);
  });

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setAudience('customer');
    setLang('th');
    sentRequests = [];
    viewResponse = MOCK_MEMBER_FULL;
    viewStatus = 200;
    viewDelayGate = null;

    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const urlStr = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(urlStr, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }

      sentRequests.push({
        method,
        url: urlStr,
        path: u.pathname,
        search: u.search,
        body,
      });

      if (u.pathname === '/api/v3/views/my_membership') {
        if (viewDelayGate) {
          await viewDelayGate;
        }
        if (viewStatus !== 200) {
          return new Response(JSON.stringify(viewResponse), {
            status: viewStatus,
            headers: { 'content-type': 'application/json' },
          });
        }
        return new Response(
          JSON.stringify(viewResponse),
          {
            status: 200,
            headers: { 'content-type': 'application/json' },
          },
        );
      }

      // Fallback for unexpected paths: return sensitive PII to ensure screen does not fetch or spread other endpoints (#1649 review item 1)
      return new Response(
        JSON.stringify({
          phone: '+66812345678',
          nationalPhone: '0812345678',
          identityId: '00000000-0000-0000-0000-000000000099',
          customerNo: 'C10001',
          passport: 'AA1234567',
        }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 20));
  }

  // 1. §7 [required]: data only from view my_membership with no phone / customer id in query or body
  it('1. asks GET /api/v3/views/my_membership with no customerId or phone in query or body', async () => {
    dispose = render(() => <MemberScreen />, host);
    await settle();

    // Must fetch ONLY /api/v3/views/my_membership (data only from view my_membership, #1649 review item 1)
    expect(sentRequests.map((r) => r.path)).toEqual(['/api/v3/views/my_membership']);

    const call = sentRequests[0];
    expect(call, 'requests /api/v3/views/my_membership').toBeDefined();
    expect(call.method).toBe('GET');

    const u = new URL(call.url, 'http://localhost');
    expect(u.searchParams.get('customer')).toBeNull();
    expect(u.searchParams.get('customerId')).toBeNull();
    expect(u.searchParams.get('phone')).toBeNull();
    expect(call.body).toBeNull();
  });

  // 2. §7 [required] & §9: pointsUnavailable: true shows tier from mirror, points worded unavailable, never 0, never error page (D-9)
  it('2. Loyalty down (pointsUnavailable: true) shows mirror tier, worded unavailable points, never 0, never an error page', async () => {
    viewResponse = MOCK_LOYALTY_DOWN;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    // Must show the mirror tier
    expect(host.textContent).toContain('PLATINUM');

    // Must show worded points unavailable
    expect(host.textContent).toContain(table.th.pointsUnavailable);

    // Points must NEVER be 0 or 0.00 (assert specifically on points container, #1649 review item 2)
    const pointsContainer = host.querySelector('.xc-member-points');
    expect(pointsContainer, '.xc-member-points element exists').not.toBeNull();
    expect(pointsContainer!.textContent).not.toMatch(/\b0\s*(คะแนน|points)?\b/i);

    // Must NOT be an error page
    expect(host.querySelector('.xc-member-error')).toBeNull();
    expect(host.textContent).not.toContain(table.th.loadFailed);
  });

  // 3. §7 [required] & §9: tier: null shows "not a member yet" with how to join, not an error
  it('3. unknown member (tier: null) shows "not a member yet" with how to join, not an error', async () => {
    viewResponse = MOCK_NOT_MEMBER;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    // Must show not a member message
    expect(host.textContent).toContain(table.th.notMember);
    expect(host.textContent).toContain(table.th.howToJoin);

    // Must not be an error page
    expect(host.querySelector('.xc-member-error')).toBeNull();
    expect(host.textContent).not.toContain(table.th.loadFailed);
  });

  // 4. §9: nextExpiry: null renders no expiry line; when present renders expiry details
  it('4. nextExpiry: null omits expiry line; when present renders expiring points and date', async () => {
    // 4a. When nextExpiry is present
    viewResponse = MOCK_MEMBER_FULL;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-expiry')).not.toBeNull();
    expect(host.textContent).toContain('50');

    // 4b. When nextExpiry is null
    dispose();
    host.innerHTML = '';
    viewResponse = {
      ...MOCK_MEMBER_FULL,
      nextExpiry: null,
    };
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-expiry')).toBeNull();
  });

  // 5. §9: next: null (top tier) omits "to next tier" bar; normal tier renders current / next
  it('5. top tier (progress.next: null) omits "to next tier" bar; normal progress renders current and next without recomputing', async () => {
    // 5a. Normal progress with next
    viewResponse = MOCK_MEMBER_FULL;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain('12500.00');
    expect(host.textContent).toContain('20000.00');
    expect(host.querySelector('.xc-member-progress-next')).not.toBeNull();

    // 5b. Top tier (next: null)
    dispose();
    host.innerHTML = '';
    viewResponse = MOCK_TOP_TIER;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain('DIAMOND');
    expect(host.querySelector('.xc-member-progress-next')).toBeNull();
  });

  // 6. §7 [required]: pending points are kept separate from available (never added together D-7)
  it('6. pending points are kept separate from available and never added together (D-7)', async () => {
    viewResponse = MOCK_PENDING_SEPARATE;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    // Available points = 500
    const availableEl = host.querySelector('.xc-member-points-available');
    expect(availableEl?.textContent ?? host.textContent).toContain('500');

    // Pending points = 120
    const pendingEl = host.querySelector('.xc-member-points-pending');
    expect(pendingEl?.textContent ?? host.textContent).toContain('120');

    // The combined sum 620 must NEVER appear in the DOM
    expect(host.textContent).not.toContain('620');
  });

  // 7. §7 [required]: no phone number or identity value appears in the DOM
  it('7. no phone number or identity value appears anywhere in the DOM', async () => {
    viewResponse = MOCK_MEMBER_FULL;
    dispose = render(() => <MemberScreen />, host);
    await settle();

    // Screen must render member data (fails on empty stub, non-vacuous, #1649 review item 1)
    expect(host.textContent).toContain('GOLD');

    // Must have requested only my_membership
    expect(sentRequests.map((r) => r.path)).toEqual(['/api/v3/views/my_membership']);

    const domHtml = host.innerHTML;
    const domText = host.textContent ?? '';

    // No E.164 phone numbers
    expect(domHtml).not.toMatch(/\+66\d+/);
    expect(domText).not.toMatch(/\+66\d+/);

    // No national phone patterns
    expect(domHtml).not.toMatch(/08\d{8}/);
    expect(domText).not.toMatch(/08\d{8}/);

    // No UUID identity strings
    expect(domHtml).not.toMatch(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i);
  });

  // 8. §8: handles outOfScope worded, server errors with retry action
  it('8. handles outOfScope worded, and server error with retry button', async () => {
    // 8a. outOfScope 403
    viewStatus = 403;
    viewResponse = { error: { code: 'outOfScope', message: 'Forbidden' } };
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain(table.th.outOfScope);

    // 8b. Server error 500 with retry
    dispose();
    host.innerHTML = '';
    viewStatus = 500;
    viewResponse = { error: { code: 'serverError', message: 'Internal error' } };
    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-error')).not.toBeNull();
    const retryBtn = host.querySelector('.xc-member-retry-btn') as HTMLButtonElement;
    expect(retryBtn, 'retry button exists').not.toBeNull();
    expect(retryBtn.textContent).toContain(table.th.retry);

    // Clicking retry refetches
    viewStatus = 200;
    viewResponse = MOCK_MEMBER_FULL;
    retryBtn.click();
    await settle();

    expect(host.textContent).toContain('GOLD');
    expect(host.querySelector('.xc-member-error')).toBeNull();
  });

  // 9. Loading state while fetching
  it('9. renders loading state while view request is pending', async () => {
    viewDelayGate = new Promise(() => {}); // never resolves
    dispose = render(() => <MemberScreen />, host);
    await settle();

    const content = host.textContent ?? '';
    const hasLoading = content.includes(table.th.loading) || host.querySelector('.xc-member-loading') !== null;
    expect(hasLoading, 'shows loading state').toBe(true);
  });

  // 10. Page and entry conventions (APP-19 contract)
  it('10. web/pages/app-member.html and main.tsx fulfill shell, gate and audience contract', () => {
    const webRoot = path.resolve(thisDir, '../../../../');
    const htmlPath = path.join(webRoot, 'pages/app-member.html');
    const mainPath = path.join(thisDir, 'main.tsx');

    expect(fs.existsSync(htmlPath), 'pages/app-member.html exists').toBe(true);
    expect(fs.existsSync(mainPath), 'main.tsx exists').toBe(true);

    const htmlContent = fs.readFileSync(htmlPath, 'utf-8');
    expect(htmlContent).toContain('<title>XCHANGE — สมาชิก | Membership</title>');
    expect(htmlContent).toMatch(/meta\s+name=["']xc-path["']\s+content=["']\/app\/member["']/);
    expect(htmlContent).toMatch(/meta\s+name=["']xc-gate["']\s+content=["']anyone["']/);
    expect(htmlContent).toMatch(/<script\s+type=["']module["']\s+src=["']\/src\/screens\/customer\/member\/main\.tsx["']/);

    const mainContent = fs.readFileSync(mainPath, 'utf-8');
    expect(mainContent).toMatch(/setAudience\(['"]customer['"]\)/);
    expect(mainContent).toMatch(/import\s+\{[^}]*\bCustomerShell\b[^}]*\}\s*from\s+['"][^'"]*_shell\/Shell['"]/);
    expect(mainContent).toMatch(/<CustomerShell\s+route=\{?['"]\/app\/member['"]\}?/);
    expect(mainContent).not.toMatch(/import\s*\{[^}]*\bAppShell\b[^}]*\}/);
    expect(mainContent).not.toMatch(/<LangSwitch/);
  });

  // 11. §9: TH/EN/ZH translation key parity
  it('11. TH, EN and ZH translation tables have exact key parity and matching placeholders', () => {
    const thKeys = Object.keys(table.th).sort();
    const enKeys = Object.keys(table.en).sort();
    const zhKeys = Object.keys(table.zh).sort();

    expect(thKeys).toEqual(enKeys);
    expect(thKeys).toEqual(zhKeys);
    expect(thKeys.length).toBeGreaterThan(0);

    for (const key of thKeys) {
      const thVal = table.th[key as keyof typeof table.th];
      const enVal = table.en[key as keyof typeof table.en];
      const zhVal = table.zh[key as keyof typeof table.zh];

      expect(typeof thVal).toBe('string');
      expect(typeof enVal).toBe('string');
      expect(typeof zhVal).toBe('string');

      expect(thVal.trim().length, `th.${key} non-empty`).toBeGreaterThan(0);
      expect(enVal.trim().length, `en.${key} non-empty`).toBeGreaterThan(0);
      expect(zhVal.trim().length, `zh.${key} non-empty`).toBeGreaterThan(0);

      const extract = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      expect(extract(thVal), `placeholder parity for ${key}`).toEqual(extract(enVal));
      expect(extract(thVal), `placeholder parity for ${key}`).toEqual(extract(zhVal));
    }
  });
});
