import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import type { MyMembershipData } from '../../../shared/api/generated/my_membership';
import { MemberScreen } from './Member';

const BASE_MEMBER: MyMembershipData = {
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
};

describe('108-Plaza/xchange#2369: Customer MemberScreen Bangkok localized dates', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let viewResponse: MyMembershipData;

  beforeEach(() => {
    vi.stubEnv('TZ', 'UTC');
    host = document.createElement('div');
    document.body.appendChild(host);
    setAudience('customer');
    setLang('th');
    viewResponse = structuredClone(BASE_MEMBER);

    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const urlStr = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(urlStr, 'http://localhost');
      if (u.pathname === '/api/v3/views/my_membership') {
        return new Response(JSON.stringify(viewResponse), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({}), { status: 404 });
    });
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    vi.unstubAllGlobals();
    vi.unstubAllEnvs();
  });

  async function settle(): Promise<void> {
    await new Promise((r) => setTimeout(r, 20));
  }

  it("Row 1: th, nextExpiry.at='2026-12-31T23:59:59Z' -> Expiry line contains '1 มกราคม 2570'; no 'T', no 'Z', no '2026-12-31'", async () => {
    setLang('th');
    viewResponse = {
      ...BASE_MEMBER,
      nextExpiry: { points: 50, at: '2026-12-31T23:59:59Z' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const expiryEl = host.querySelector('.xc-member-expiry');
    expect(expiryEl).not.toBeNull();
    const text = expiryEl!.textContent ?? '';
    expect(text).toContain('50');
    expect(text).toContain('1 มกราคม 2570');
    expect(text).not.toContain('T');
    expect(text).not.toContain('Z');
    expect(text).not.toContain('2026-12-31');
  });

  it("Row 2: en, effectiveUntil='2026-11-15T00:00:00Z' -> 'Effective until 15 November 2026'", async () => {
    setLang('en');
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: '2026-11-15T00:00:00Z',
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const untilEl = host.querySelector('.xc-member-effective-until');
    expect(untilEl).not.toBeNull();
    expect(untilEl!.textContent?.trim()).toBe('Effective until 15 November 2026');
  });

  it("Row 3: en, effectiveUntil='2026-12-31T18:30:00Z' -> '1 January 2027' (the Bangkok day)", async () => {
    setLang('en');
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: '2026-12-31T18:30:00Z',
      nextExpiry: { points: 50, at: '2026-12-31T18:30:00Z' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const untilEl = host.querySelector('.xc-member-effective-until');
    expect(untilEl).not.toBeNull();
    expect(untilEl!.textContent?.trim()).toBe('Effective until 1 January 2027');

    const expiryEl = host.querySelector('.xc-member-expiry');
    expect(expiryEl).not.toBeNull();
    expect(expiryEl!.textContent?.trim()).toBe('50 points expiring on 1 January 2027');
  });

  it("Edge case 1: effectiveUntil: null / nextExpiry: null render no line while present date is formatted", async () => {
    setLang('th');
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: null,
      nextExpiry: { points: 50, at: '2026-12-31T23:59:59Z' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-effective-until')).toBeNull();
    expect(host.querySelector('.xc-member-expiry')?.textContent).toContain('1 มกราคม 2570');

    dispose();
    host.innerHTML = '';
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: '2026-11-15T00:00:00Z',
      nextExpiry: null,
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-expiry')).toBeNull();
    expect(host.querySelector('.xc-member-effective-until')?.textContent).toContain('15 พฤศจิกายน 2569');
  });

  it("Edge case 2: zh renders '2026年12月31日' for '2026-12-31T16:59:59Z'", async () => {
    setLang('zh');
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: '2026-12-31T16:59:59Z',
      nextExpiry: { points: 50, at: '2026-12-31T16:59:59Z' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const untilEl = host.querySelector('.xc-member-effective-until');
    expect(untilEl).not.toBeNull();
    expect(untilEl!.textContent?.trim()).toBe('有效期至 2026年12月31日');

    const expiryEl = host.querySelector('.xc-member-expiry');
    expect(expiryEl).not.toBeNull();
    expect(expiryEl!.textContent?.trim()).toBe('50 积分将于 2026年12月31日 过期');
  });

  it('Edge case 3: unparsable string shows the raw value, never "Invalid Date", while valid date is formatted', async () => {
    setLang('en');
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: 'not-a-valid-date',
      nextExpiry: { points: 50, at: '2026-12-31T18:30:00Z' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const untilEl = host.querySelector('.xc-member-effective-until');
    expect(untilEl?.textContent?.trim()).toBe('Effective until not-a-valid-date');
    expect(host.textContent).not.toContain('Invalid Date');
    expect(host.querySelector('.xc-member-expiry')?.textContent?.trim()).toBe(
      '50 points expiring on 1 January 2027',
    );

    dispose();
    host.innerHTML = '';
    viewResponse = {
      ...BASE_MEMBER,
      effectiveUntil: '2026-11-15T00:00:00Z',
      nextExpiry: { points: 50, at: 'unparsable-expiry' },
    };

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.querySelector('.xc-member-effective-until')?.textContent?.trim()).toBe(
      'Effective until 15 November 2026',
    );
    expect(host.querySelector('.xc-member-expiry')?.textContent?.trim()).toBe(
      '50 points expiring on unparsable-expiry',
    );
    expect(host.textContent).not.toContain('Invalid Date');
  });
});
