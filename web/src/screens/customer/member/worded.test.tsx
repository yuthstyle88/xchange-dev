import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import type { MyMembershipData } from '../../../shared/api/generated/my_membership';
import { setAudience, setLang } from '../../../shared/i18n';
import { en as sharedEn } from '../../../shared/i18n/en';
import { th as sharedTh } from '../../../shared/i18n/th';
import * as table from './i18n';
import { MemberScreen } from './Member';

const BARE_FULL_MEMBER = {
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

const BARE_LOYALTY_DOWN = {
  tier: 'PLATINUM',
  effectiveUntil: '2026-11-15T00:00:00Z',
  points: null,
  pointsUnavailable: true,
  nextExpiry: null,
  progress: null,
} satisfies MyMembershipData;

const BARE_NO_TIER_WITH_POINTS = {
  tier: null,
  effectiveUntil: null,
  points: {
    available: 80,
    pending: 10,
  },
  pointsUnavailable: false,
  nextExpiry: null,
  progress: null,
} satisfies MyMembershipData;

const BARE_NO_TIER_WITH_EXPIRING_POINTS = {
  tier: null,
  effectiveUntil: null,
  points: {
    available: 0,
    pending: 0,
  },
  pointsUnavailable: false,
  nextExpiry: {
    points: 30,
    at: '2026-12-31T23:59:59Z',
  },
  progress: null,
} satisfies MyMembershipData;

const BARE_TOP_TIER = {
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

describe('APP-19 / C05: bare MyMembershipData & wordedError handling (#1649)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
    setAudience('customer');
    setLang('th');
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

  it('renders bare MyMembershipData object from /api/v3/views/my_membership without envelope', async () => {
    vi.stubGlobal('fetch', async () =>
      new Response(JSON.stringify(BARE_FULL_MEMBER), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain('GOLD');
    expect(host.querySelector('.xc-member-points-available')?.textContent).toContain('100');
    expect(host.querySelector('.xc-member-points-pending')?.textContent).toContain('25');
    expect(host.querySelector('.xc-member-points')?.textContent).not.toContain('125');
    expect(host.querySelector('.xc-member-expiry')?.textContent).toContain('50');
    expect(host.querySelector('.xc-member-progress-next')?.textContent).toContain('12500.00');
    expect(host.querySelector('.xc-member-progress-next')?.textContent).toContain('20000.00');
  });

  it('renders bare loyalty-down and top-tier fixtures typed with satisfies MyMembershipData', async () => {
    vi.stubGlobal('fetch', async () =>
      new Response(JSON.stringify(BARE_LOYALTY_DOWN), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain('PLATINUM');
    expect(host.querySelector('.xc-member-points')?.textContent).toContain(
      table.th.pointsUnavailable,
    );
    expect(host.querySelector('.xc-member-error')).toBeNull();

    dispose();
    host.innerHTML = '';

    vi.stubGlobal('fetch', async () =>
      new Response(JSON.stringify(BARE_TOP_TIER), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain('DIAMOND');
    expect(host.textContent).toContain(table.th.topTier);
    expect(host.querySelector('.xc-member-progress-next')).toBeNull();
  });

  it('tier: null with non-zero available, pending or expiring points shows points and neutral no-tier line WITHOUT howToJoin prompt', async () => {
    vi.stubGlobal('fetch', async () =>
      new Response(JSON.stringify(BARE_NO_TIER_WITH_POINTS), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain(table.th.notMember);
    expect(host.textContent).not.toContain(table.th.howToJoin);
    expect(host.querySelector('.xc-member-how-to-join')).toBeNull();
    expect(host.querySelector('.xc-member-points-available')?.textContent).toContain('80');
    expect(host.querySelector('.xc-member-points-pending')?.textContent).toContain('10');
    expect(host.textContent).not.toContain('90');

    dispose();
    host.innerHTML = '';

    vi.stubGlobal('fetch', async () =>
      new Response(JSON.stringify(BARE_NO_TIER_WITH_EXPIRING_POINTS), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      }),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    expect(host.textContent).toContain(table.th.notMember);
    expect(host.textContent).not.toContain(table.th.howToJoin);
    expect(host.querySelector('.xc-member-how-to-join')).toBeNull();
    expect(host.querySelector('.xc-member-expiry')?.textContent).toContain('30');
  });

  it('(1) network TypeError shows error.network in TH, and after setLang("en") shows error.network in EN', async () => {
    vi.stubGlobal('fetch', async () => {
      throw new TypeError('Failed to fetch');
    });

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const errEl = host.querySelector('.xc-member-error');
    expect(errEl).not.toBeNull();
    expect(errEl?.textContent).toContain(sharedTh['error.network']);

    dispose();
    host.innerHTML = '';
    setLang('en');

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const errElEn = host.querySelector('.xc-member-error');
    expect(errElEn).not.toBeNull();
    expect(errElEn?.textContent).toContain(sharedEn['error.network']);
  });

  it('(2) a 500 refusal with keyed non-server code roleNotAllowedHere shows th["error.serverError"]', async () => {
    vi.stubGlobal('fetch', async () =>
      new Response(
        JSON.stringify({
          error: { code: 'roleNotAllowedHere', message: 'Internal loyalty mirror failure' },
        }),
        { status: 500, headers: { 'content-type': 'application/json' } },
      ),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const errMsg = host.querySelector('.xc-member-error-message');
    expect(errMsg?.textContent).toBe(sharedTh['error.serverError']);
  });

  it('(3) a refusal with an unknown code and server message shows that server message', async () => {
    const serverSentence = 'ระบบสมาชิกกำลังปรับปรุงชั่วคราว กรุณารอสักครู่';
    vi.stubGlobal('fetch', async () =>
      new Response(
        JSON.stringify({
          error: { code: 'membershipLedgerMaintenance', message: serverSentence },
        }),
        { status: 422, headers: { 'content-type': 'application/json' } },
      ),
    );

    dispose = render(() => <MemberScreen />, host);
    await settle();

    const errMsg = host.querySelector('.xc-member-error-message');
    expect(errMsg?.textContent).toBe(serverSentence);
  });
});
