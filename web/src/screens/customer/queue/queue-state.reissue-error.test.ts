// Up-front tests for SF-b, the reload half (docs/design/web-silent-failures.md §2 SF-3, B2). Do not edit, delete or
// weaken; add tests in a file of your own.
//
// A reload (`?request=<id>`) reissues the QR. Only a 404 means the request is gone (the form, as today). A 401 means
// signed out (the U1 path). Anything else is phase `reissueFailed` with a worded error and `retryReissue()`, which
// reissues the same id again; the page keeps `request=<id>`. Never the empty form. The harness is
// queue-state.test.ts's, copied.
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { en, th } from './i18n';
import type { CreateResult, OpenedRequest, QueueApi, QueueProfile, QueueRates, WebRequestInput } from './queue-api';
import { createQueueRequestState, type QueueRequestState } from './queue-state';

const EXPIRES = '2026-10-21T21:00:00Z';
const RATES: QueueRates = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};
const PROFILE: QueueProfile = {
  customerId: 'c-1',
  displayName: 'Somchai',
  emailCopy: { available: true, to: 'j***@example.com' },
};
const FORM = { type: 'BUY' as const, currency: 'USD', amount: '100' };
const RELOAD = '?branch=MAIN&request=k-1';

function refusal(status: number, code: string, message = code): ApiError {
  return new ApiError({ code, message, status });
}

function fakeApi() {
  return {
    client: new ApiClient(async () => new Response('{}', { status: 200 })),
    host: vi.fn(async (): Promise<string> => 'ORG_1'),
    rates: vi.fn(async (_org: string, _code: string): Promise<QueueRates> => RATES),
    profile: vi.fn(async (): Promise<QueueProfile> => PROFILE),
    create: vi.fn(
      async (_input: WebRequestInput): Promise<CreateResult> => ({
        kind: 'opened',
        request: { requestId: 'k-1', token: 'tok-1', expiresAt: EXPIRES },
      }),
    ),
    reissueQr: vi.fn(
      async (_branchId: string, requestId: string): Promise<OpenedRequest> => ({
        requestId,
        token: 'tok-fresh',
        expiresAt: EXPIRES,
      }),
    ),
    cancel: vi.fn(async (_branchId: string, requestId: string) => ({ requestId, status: 'CANCELLED' as const })),
    emailCopy: vi.fn(async () => ({ sent: true })),
  };
}

let fake: ReturnType<typeof fakeApi>;
let disposeRoot: (() => void) | undefined;

function make(search = RELOAD): QueueRequestState {
  return createRoot((dispose) => {
    disposeRoot = dispose;
    return createQueueRequestState(fake as unknown as QueueApi, search);
  });
}

function pageParam(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

const FAILED = { kind: 'reissueFailed', requestId: 'k-1', message: th.reissueFailed };

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  fake = fakeApi();
  window.history.replaceState(null, '', `/app/queue${RELOAD}`);
});

afterEach(() => {
  disposeRoot?.();
  disposeRoot = undefined;
  window.history.replaceState(null, '', '/');
});

describe('SF-b: reload, the reissue answers 404 (guard: the form, as today)', () => {
  it('404 ⇒ the form, and the page URL drops request=', async () => {
    fake.reissueQr.mockRejectedValue(refusal(404, 'notFound'));
    const s = make();
    expect(await s.load()).toEqual({ signedOut: false });
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
    expect(pageParam('request')).toBeNull();
  });
});

describe('SF-b: reload, the reissue answers 401', () => {
  it('401 ⇒ {signedOut: true}, not the form, and nothing can be sent', async () => {
    fake.reissueQr.mockRejectedValue(refusal(401, 'signIn'));
    const s = make();
    expect(await s.load()).toEqual({ signedOut: true });
    expect(s.phase().kind).not.toBe('form');
    await s.submit(FORM);
    expect(fake.create).not.toHaveBeenCalled();
  });
});

describe('SF-b: reload, the reissue fails otherwise', () => {
  it('500 ⇒ phase reissueFailed with the worded text and the request id; never the form', async () => {
    fake.reissueQr.mockRejectedValue(refusal(500, 'serverError'));
    const s = make();
    expect(await s.load()).toEqual({ signedOut: false });
    expect(s.phase()).toEqual(FAILED);
    expect(pageParam('request'), 'the page keeps request=<id> so a reload finds it').toBe('k-1');
    expect(pageParam('branch')).toBe('MAIN');
  });

  it('a network failure (not an ApiError) ⇒ the same reissueFailed phase', async () => {
    fake.reissueQr.mockRejectedValue(new TypeError('Failed to fetch'));
    const s = make();
    expect(await s.load()).toEqual({ signedOut: false });
    expect(s.phase()).toEqual(FAILED);
    expect(pageParam('request')).toBe('k-1');
  });

  it('the text is in the page language', async () => {
    setLang('en');
    fake.reissueQr.mockRejectedValue(refusal(502, 'httpError'));
    const s = make();
    await s.load();
    expect(s.phase()).toEqual({ ...FAILED, message: en.reissueFailed });
  });
});

describe('SF-b: retryReissue()', () => {
  it('reissues the same request once more, and opens it on success', async () => {
    fake.reissueQr.mockRejectedValueOnce(refusal(500, 'serverError'));
    const s = make();
    await s.load();
    expect(fake.reissueQr).toHaveBeenCalledTimes(1);
    expect(await s.retryReissue()).toEqual({ signedOut: false });
    expect(fake.reissueQr).toHaveBeenCalledTimes(2);
    expect(fake.reissueQr).toHaveBeenLastCalledWith('b-id-7', 'k-1');
    expect(s.phase()).toEqual({
      kind: 'open',
      request: { requestId: 'k-1', token: 'tok-fresh', expiresAt: EXPIRES },
      status: null,
    });
    expect(pageParam('request')).toBe('k-1');
    expect(fake.create).not.toHaveBeenCalled();
  });

  it('a retry that fails again stays reissueFailed with the same id', async () => {
    fake.reissueQr.mockRejectedValue(refusal(503, 'serverError'));
    const s = make();
    await s.load();
    await s.retryReissue();
    expect(fake.reissueQr).toHaveBeenCalledTimes(2);
    expect(s.phase()).toEqual(FAILED);
    expect(pageParam('request')).toBe('k-1');
  });

  it('a retry answered 404 ⇒ the form (the request is gone)', async () => {
    fake.reissueQr.mockRejectedValueOnce(refusal(500, 'serverError')).mockRejectedValueOnce(refusal(404, 'notFound'));
    const s = make();
    await s.load();
    expect(s.phase()).toEqual(FAILED);
    expect(await s.retryReissue()).toEqual({ signedOut: false });
    expect(fake.reissueQr).toHaveBeenCalledTimes(2);
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
    expect(pageParam('request')).toBeNull();
  });

  it('a retry answered 401 ⇒ {signedOut: true}', async () => {
    fake.reissueQr.mockRejectedValueOnce(refusal(500, 'serverError')).mockRejectedValueOnce(refusal(401, 'signIn'));
    const s = make();
    await s.load();
    expect(await s.retryReissue()).toEqual({ signedOut: true });
    expect(s.phase().kind).not.toBe('form');
    await s.submit(FORM);
    expect(fake.create).not.toHaveBeenCalled();
  });

  it('outside reissueFailed it sends nothing', async () => {
    const s = make('?branch=MAIN');
    await s.load();
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
    await s.retryReissue();
    expect(fake.reissueQr).not.toHaveBeenCalled();
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
  });
});
