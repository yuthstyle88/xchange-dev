// Up-front tests for QW-2b (kiosk-shared-phone §5r.4a). Do not edit, delete or weaken; add tests in a file of your own.
//
// QueueRequestState, the /app/queue screen's state. Rows: U1, U2, U4, U9, the reload and the expiry of §5r.3 / F8.
// Only the state is under test: QueueApi is a fake with QueueApi's signature (queue-api.ts), so this file does not
// depend on QW-2a's bodies.
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { setAudience, setLang } from '../../../shared/i18n';
import { th } from './i18n';
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

function make(search = '?branch=MAIN'): QueueRequestState {
  return createRoot((dispose) => {
    disposeRoot = dispose;
    return createQueueRequestState(fake as unknown as QueueApi, search);
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

function pageParam(name: string): string | null {
  return new URLSearchParams(window.location.search).get(name);
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  fake = fakeApi();
  window.history.replaceState(null, '', '/app/queue?branch=MAIN');
});

afterEach(() => {
  disposeRoot?.();
  disposeRoot = undefined;
  window.history.replaceState(null, '', '/');
});

describe('QW-2b: load', () => {
  it('U1: my_profile 401 resolves {signedOut: true}, and nothing can be sent afterwards', async () => {
    fake.profile.mockRejectedValue(refusal(401, 'signIn'));
    const s = make();
    expect(await s.load()).toEqual({ signedOut: true });
    await s.submit(FORM);
    expect(fake.create).not.toHaveBeenCalled();
  });

  it('signed in: the branch code becomes the branch id; the form shows; emailCopy comes from my_profile', async () => {
    const s = make();
    expect(s.branchCode).toBe('MAIN');
    expect(await s.load()).toEqual({ signedOut: false });
    expect(fake.rates).toHaveBeenCalledWith('ORG_1', 'MAIN');
    expect(s.branchId()).toBe('b-id-7');
    expect(s.rates().map((r) => r.code)).toEqual(['USD']);
    expect(s.emailCopy()).toEqual({ available: true, to: 'j***@example.com' });
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
  });

  it('no ?branch=: phase failed with the worded noBranch text, and no read of rates', async () => {
    const s = make('');
    await s.load();
    expect(s.phase()).toEqual({ kind: 'failed', message: th.noBranch });
    expect(fake.rates).not.toHaveBeenCalled();
  });

  it('a failed rates read: phase failed with the worded loadFailed text', async () => {
    fake.rates.mockRejectedValue(refusal(404, 'notFound'));
    const s = make();
    await s.load();
    expect(s.phase()).toEqual({ kind: 'failed', message: th.loadFailed });
  });

  it('reload: ?request=<id> reissues a fresh QR for that request and opens it; nothing is created', async () => {
    const s = make('?branch=MAIN&request=k-1');
    await s.load();
    expect(fake.reissueQr).toHaveBeenCalledTimes(1);
    expect(fake.reissueQr).toHaveBeenCalledWith('b-id-7', 'k-1');
    expect(fake.create).not.toHaveBeenCalled();
    expect(s.phase()).toEqual({
      kind: 'open',
      request: { requestId: 'k-1', token: 'tok-fresh', expiresAt: EXPIRES },
      status: null,
    });
  });

  it('reload: a refused reissue (404, the request is gone) goes back to the form', async () => {
    fake.reissueQr.mockRejectedValue(refusal(404, 'notFound'));
    const s = make('?branch=MAIN&request=k-1');
    await s.load();
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
  });
});

describe('QW-2b: submit', () => {
  it('U2: one create with the branch id and the form; the request opens with its token and no status yet', async () => {
    const s = make();
    await s.load();
    await s.submit(FORM);
    expect(fake.create).toHaveBeenCalledTimes(1);
    expect(fake.create).toHaveBeenCalledWith({ branch: 'b-id-7', type: 'BUY', currency: 'USD', amount: '100' });
    expect(s.phase()).toEqual({
      kind: 'open',
      request: { requestId: 'k-1', token: 'tok-1', expiresAt: EXPIRES },
      status: null,
    });
    expect(s.busy()).toBe(false);
    expect(s.error()).toBeNull();
  });

  it('U2: a second submit while the first is in flight sends nothing', async () => {
    let release!: (r: CreateResult) => void;
    fake.create.mockImplementation(
      () =>
        new Promise<CreateResult>((resolve) => {
          release = resolve;
        }),
    );
    const s = make();
    await s.load();
    const first = s.submit(FORM);
    await flush();
    expect(s.busy()).toBe(true);
    await s.submit(FORM);
    release({ kind: 'opened', request: { requestId: 'k-1', token: 'tok-1', expiresAt: EXPIRES } });
    await first;
    expect(fake.create).toHaveBeenCalledTimes(1);
  });

  it('U9: alreadyQueued reissues the open request and opens it with the fresh token', async () => {
    fake.create.mockResolvedValue({ kind: 'alreadyQueued', requestId: 'k-old' });
    const s = make();
    await s.load();
    await s.submit(FORM);
    expect(fake.reissueQr).toHaveBeenCalledWith('b-id-7', 'k-old');
    expect(s.phase()).toEqual({
      kind: 'open',
      request: { requestId: 'k-old', token: 'tok-fresh', expiresAt: EXPIRES },
      status: null,
    });
  });

  it('a replayed create (no token) reissues the request for a token', async () => {
    fake.create.mockResolvedValue({ kind: 'replayed', requestId: 'k-1', expiresAt: EXPIRES });
    const s = make();
    await s.load();
    await s.submit(FORM);
    expect(fake.reissueQr).toHaveBeenCalledWith('b-id-7', 'k-1');
    const p = s.phase();
    expect(p.kind === 'open' ? p.request.token : null).toBe('tok-fresh');
  });

  it('a refused create keeps the form and words the refusal', async () => {
    fake.create.mockRejectedValue(refusal(400, 'quoteRefused', 'สกุลเงินนี้ปิดรับ'));
    const s = make();
    await s.load();
    await s.submit(FORM);
    expect(s.phase()).toEqual({ kind: 'form', expired: false });
    expect(s.error()).toBeTruthy();
    expect(s.busy()).toBe(false);
  });

  it('an open request is written into the page URL as request=<id> (branch kept), so a reload finds it', async () => {
    const s = make();
    await s.load();
    await s.submit(FORM);
    expect(pageParam('request')).toBe('k-1');
    expect(pageParam('branch')).toBe('MAIN');
  });
});

describe('QW-2b: cancel and expiry', () => {
  it('U4: cancel sends one cancel for this request and branch; the status becomes CANCELLED', async () => {
    const s = make();
    await s.load();
    await s.submit(FORM);
    await Promise.all([s.cancel(), s.cancel()]);
    expect(fake.cancel).toHaveBeenCalledTimes(1);
    expect(fake.cancel).toHaveBeenCalledWith('b-id-7', 'k-1');
    const p = s.phase();
    expect(p.kind).toBe('open');
    expect(p.kind === 'open' ? p.status : null).toBe('CANCELLED');
  });

  it('expiry: expire() goes back to the form with the expired notice, and drops request= from the URL', async () => {
    const s = make();
    await s.load();
    await s.submit(FORM);
    s.expire();
    expect(s.phase()).toEqual({ kind: 'form', expired: true });
    expect(pageParam('request')).toBeNull();
    expect(pageParam('branch')).toBe('MAIN');
  });
});
