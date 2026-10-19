// #2357: Staff RATES card — out-of-order retry sequence guard and in-flight retry click guard (§8, §9).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { errorText, setLang } from '../../../shared/i18n';
import {
  makePublishedLineDto,
  makePublishedPublicationDto,
  makePublishedRatesData,
  type PublishedRatesData,
} from '../../../shared/testing/fixtures';
import { card } from './card';

type Me = {
  userId: string;
  name: string;
  roles: { role: string; branchId?: string }[];
  branchId: string;
  branchName: string;
  permissions: string[];
};

const BRANCH = 'MAIN';

function me() {
  return {
    userId: '00000000-0000-4000-8000-000000000001',
    name: 'คุณทดสอบ',
    roles: [],
    branchId: BRANCH,
    branchName: 'สาขาทดสอบ',
    permissions: ['CompatRead'],
  };
}

function ctx(m: Me) {
  return { me: m, floor: undefined, refresh: vi.fn(async () => {}) } as unknown as Parameters<
    typeof card.visible
  >[0];
}

const RATES = makePublishedRatesData({
  publication: makePublishedPublicationDto({ id: '11111111-2222-4333-8444-555555555555', version: 7, publishedAt: '2026-10-10T03:15:00Z' }),
  sourceDate: '2026-10-09',
  stale: false,
  lines: [
    makePublishedLineDto({ currency: 'USD', group: 'ALL', unitSize: '1', buyRate: '32.4500', sellRate: '32.9000', lineId: 'line-usd' }),
    makePublishedLineDto({ currency: 'JPY', group: 'ALL', unitSize: '100', buyRate: '21.0450', sellRate: '21.5926', lineId: 'line-jpy' }),
  ],
});

const RATE_STRINGS = RATES.lines.flatMap((l) => [l.buyRate, l.sellRate]).filter((r): r is string => r !== null);

interface PendingRead {
  resolve: (res: Response) => void;
  reject: (err: unknown) => void;
}

function okResponse(data: PublishedRatesData): Response {
  return new Response(JSON.stringify({ version: 3, changed: true, data }), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function errorResponse(status: number, code: string, message: string): Response {
  return new Response(JSON.stringify({ error: { code, message } }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

async function settle(): Promise<void> {
  for (let i = 0; i < 6; i++) {
    await new Promise((r) => setTimeout(r, 0));
  }
}

describe('S-8b RatesCard retry ordering and in-flight guard (#2357)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;
  let pendingReads: PendingRead[];

  beforeEach(() => {
    pendingReads = [];
    host = document.createElement('div');
    document.body.appendChild(host);
    setLang('th');

    vi.stubGlobal(
      'fetch',
      vi.fn((input: RequestInfo | URL) => {
        const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
        if (!url.startsWith('/api/v3/views/staff_published_rates')) {
          return Promise.resolve(errorResponse(404, 'notFound', 'unexpected request'));
        }
        return new Promise<Response>((resolve, reject) => {
          pendingReads.push({ resolve, reject });
        });
      }),
    );
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
    setLang('th');
  });

  async function mountFailedInitial(): Promise<HTMLDivElement> {
    const C = card.Component;
    const c = ctx(me());
    dispose = render(() => <C ctx={c} />, host);
    await settle();
    expect(pendingReads.length).toBe(1);
    pendingReads[0]!.resolve(errorResponse(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    await settle();
    expect(host.querySelector('[role="alert"]')).not.toBeNull();
    return host;
  }

  it('row 1: Retry A held, retry B succeeds, then A fails -> rates shown, no error', async () => {
    const el = await mountFailedInitial();
    const retryBtn = el.querySelector<HTMLButtonElement>('button')!;

    // Start Retry A (held)
    retryBtn.click();
    await settle();
    expect(pendingReads.length).toBe(2);
    const retryA = pendingReads[1]!;

    // Start Retry B while Retry A is still held in flight
    retryBtn.disabled = false;
    retryBtn.click();
    await settle();
    expect(pendingReads.length).toBe(3);
    const retryB = pendingReads[2]!;

    // Retry B succeeds first
    retryB.resolve(okResponse(RATES));
    await settle();
    expect(el.querySelector('[role="alert"]')).toBeNull();
    for (const r of RATE_STRINGS) {
      expect(el.textContent ?? '').toContain(r);
    }

    // Retry A fails late -> must not clear rates or show the error
    retryA.resolve(errorResponse(500, 'serverError', 'เซิร์ฟเวอร์ขัดข้อง'));
    await settle();

    expect(el.querySelector('[role="alert"]'), 'late failure from Retry A must not show error').toBeNull();
    expect(el.querySelectorAll('.xc-rates-row').length).toBe(RATES.lines.length);
    for (const r of RATE_STRINGS) {
      expect(el.textContent ?? '').toContain(r);
    }
  });

  it("row 2: Retry A held, retry B fails, then A succeeds late -> B's error stays (latest wins)", async () => {
    const el = await mountFailedInitial();
    const retryBtn = el.querySelector<HTMLButtonElement>('button')!;

    // Start Retry A (held)
    retryBtn.click();
    await settle();
    expect(pendingReads.length).toBe(2);
    const retryA = pendingReads[1]!;

    // Start Retry B while Retry A is still held in flight
    retryBtn.disabled = false;
    retryBtn.click();
    await settle();
    expect(pendingReads.length).toBe(3);
    const retryB = pendingReads[2]!;

    // Retry B fails first with outOfScope (403)
    retryB.resolve(errorResponse(403, 'outOfScope', 'สาขานี้อยู่นอกสิทธิ์ของคุณ'));
    await settle();
    expect(el.querySelector('[role="alert"]')?.textContent).toContain(
      errorText('outOfScope', 'สาขานี้อยู่นอกสิทธิ์ของคุณ'),
    );

    // Retry A succeeds late with RATES -> must be ignored so B's error stays
    retryA.resolve(okResponse(RATES));
    await settle();

    const alert = el.querySelector('[role="alert"]');
    expect(alert, "B's error must stay when older Retry A succeeds late").not.toBeNull();
    expect(alert!.textContent).toContain(errorText('outOfScope', 'สาขานี้อยู่นอกสิทธิ์ของคุณ'));
    expect(el.querySelectorAll('.xc-rates-row').length).toBe(0);
    for (const r of RATE_STRINGS) {
      expect(el.textContent ?? '').not.toContain(r);
    }
  });

  it('row 3: two clicks while one read is in flight -> one request', async () => {
    const el = await mountFailedInitial();
    const retryBtn = el.querySelector<HTMLButtonElement>('button')!;
    expect(pendingReads.length).toBe(1);

    // Click twice while the retry read is in flight
    retryBtn.click();
    retryBtn.click();
    await settle();

    expect(pendingReads.length, 'only one retry request sent while in flight').toBe(2);

    pendingReads[1]!.resolve(okResponse(RATES));
    await settle();
    expect(el.querySelector('[role="alert"]')).toBeNull();
    expect(el.querySelectorAll('.xc-rates-row').length).toBe(RATES.lines.length);
  });

  it('edge cases (§9): language switch updates the active error & retry text, and unmounting while a read is in flight causes no error after unmount', async () => {
    const el = await mountFailedInitial();
    expect(el.querySelector('[role="alert"]')?.textContent).toMatch(/[฀-๿]/);

    // Language switch to English updates both the error alert and retry button text
    setLang('en');
    await settle();
    const enAlert = el.querySelector('[role="alert"]')?.textContent ?? '';
    expect(enAlert.length).toBeGreaterThan(0);
    expect(enAlert, 'error alert switches to English when setLang("en") is called').not.toMatch(/[฀-๿]/);
    const enBtn = el.querySelector('button')?.textContent ?? '';
    expect(enBtn).not.toMatch(/[฀-๿]/);

    // Start a retry read, then unmount while it is in flight; late failure causes no error
    const retryBtn = el.querySelector<HTMLButtonElement>('button')!;
    retryBtn.click();
    await settle();
    expect(pendingReads.length).toBe(2);

    dispose?.();
    dispose = undefined;

    pendingReads[1]!.reject(new TypeError('Failed to fetch after unmount'));
    await settle();
  });
});
