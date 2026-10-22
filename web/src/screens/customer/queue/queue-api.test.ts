// Up-front tests for QW-2a (kiosk-shared-phone §5r.4a). Do not edit, delete or weaken; add tests in a file of your own.
//
// QueueApi, the /app/queue page's wire calls (§4.3a W1–W5, W11; routes of slices c and d, `origin/feat/kwq-c` and
// `origin/feat/kwq-d`). Rows: U2's wire, U9, the replayed create, Idempotency-Key. Only QueueApi is under test; the
// network is an injected fetch.
import { beforeEach, describe, expect, it } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import { QueueApi } from './queue-api';

interface Sent {
  method: string;
  path: string;
  search: string;
  body: unknown;
  headers: Headers;
}

const EXPIRES = '2026-10-21T21:00:00Z';
const RATES = {
  branchId: 'b-id-7',
  branchName: 'XC-B',
  open: true,
  rates: [{ code: 'USD', name: 'ดอลลาร์', nameEn: 'US Dollar', buy: 32.1, sell: 32.6 }],
  paymentMethods: [],
};

let sent: Sent[];
let answers: Array<(s: Sent) => Response | undefined>;

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** Answers with the first handler that answers; anything else is a 404. */
function queueApi(): QueueApi {
  return new QueueApi(async (input: string, init?: RequestInit) => {
    const u = new URL(input, 'http://localhost');
    const s: Sent = {
      method: (init?.method ?? 'GET').toUpperCase(),
      path: u.pathname,
      search: u.search,
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
      headers: new Headers(init?.headers),
    };
    sent.push(s);
    for (const a of answers) {
      const r = a(s);
      if (r) return r;
    }
    return json(404, { error: { code: 'notFound', message: u.pathname } });
  });
}

function on(method: string, path: string | RegExp, answer: (s: Sent) => Response): void {
  answers.push((s) =>
    s.method === method && (typeof path === 'string' ? s.path === path : path.test(s.path)) ? answer(s) : undefined,
  );
}

const writes = () => sent.filter((s) => s.method !== 'GET');

beforeEach(() => {
  sent = [];
  answers = [];
});

describe('QW-2a: QueueApi reads', () => {
  it('W10, W2: host() is the org of /api/public/host; rates(org, code) reads queue-rates by branch code', async () => {
    on('GET', '/api/public/host', () => json(200, { org: 'ORG_1' }));
    on('GET', '/api/public/orgs/ORG_1/branches/MAIN/queue-rates', () => json(200, RATES));
    const api = queueApi();
    expect(await api.host()).toBe('ORG_1');
    const rates = await api.rates('ORG_1', 'MAIN');
    expect(rates.branchId).toBe('b-id-7');
    expect(rates.rates.map((r) => r.code)).toEqual(['USD']);
    expect(writes()).toEqual([]);
  });

  it('rates(): a refused read rejects with an ApiError', async () => {
    on('GET', '/api/public/orgs/ORG_1/branches/NOPE/queue-rates', () =>
      json(404, { error: { code: 'notFound', message: 'branch' } }),
    );
    await expect(queueApi().rates('ORG_1', 'NOPE')).rejects.toBeInstanceOf(ApiError);
  });

  it('W11: profile() carries my_profile.emailCopy; signed out rejects with ApiError status 401', async () => {
    let status = 200;
    on('GET', '/api/v3/views/my_profile', () =>
      status === 200
        ? json(200, {
            version: 1,
            changed: true,
            data: { customerId: 'c-1', displayName: 'Somchai', emailCopy: { available: true, to: 'j***@example.com' } },
          })
        : json(401, { error: { code: 'signIn', message: 'sign in' } }),
    );
    const api = queueApi();
    const me = await api.profile();
    expect(me.emailCopy).toEqual({ available: true, to: 'j***@example.com' });
    status = 401;
    const err = await api.profile().then(
      () => undefined,
      (e: unknown) => e,
    );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(401);
  });
});

describe('QW-2a: QueueApi.create (W1, W3)', () => {
  const input = { branch: 'b-id-7', type: 'BUY' as const, currency: 'USD', amount: '100' };

  it('U2 wire: posts {branch, type, currency, amount} with an Idempotency-Key; the token comes from /q#<token>', async () => {
    on('POST', '/api/app/web-requests', () =>
      json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: EXPIRES }),
    );
    const result = await queueApi().create(input);
    expect(result).toEqual({ kind: 'opened', request: { requestId: 'k-1', token: 'tok-1', expiresAt: EXPIRES } });
    expect(writes()).toHaveLength(1);
    const w = writes()[0]!;
    expect(w.path).toBe('/api/app/web-requests');
    expect(w.body).toEqual({ branch: 'b-id-7', type: 'BUY', currency: 'USD', amount: '100' });
    expect(w.headers.get('Idempotency-Key'), 'Idempotency-Key').toBeTruthy();
    expect(w.headers.get('X-Xchange-Request')).toBe('1');
  });

  it('Idempotency-Key: a retry of the same create after a 503 sends the same key', async () => {
    let first = true;
    on('POST', '/api/app/web-requests', () => {
      if (first) {
        first = false;
        return json(503, { error: { code: 'serverError', message: 'down' } });
      }
      return json(200, { requestId: 'k-1', requestQrUrl: '/q#tok-1', expiresAt: EXPIRES });
    });
    const api = queueApi();
    await expect(api.create(input)).rejects.toBeInstanceOf(ApiError);
    await api.create(input);
    expect(writes()).toHaveLength(2);
    const [a, b] = writes().map((w) => w.headers.get('Idempotency-Key'));
    expect(a).toBeTruthy();
    expect(b).toBe(a);
  });

  it('replay: an answer with requestQrUrl null is "replayed" with no token, so the caller reissues', async () => {
    on('POST', '/api/app/web-requests', () => json(200, { requestId: 'k-1', requestQrUrl: null, expiresAt: EXPIRES }));
    expect(await queueApi().create(input)).toEqual({ kind: 'replayed', requestId: 'k-1', expiresAt: EXPIRES });
  });

  it('U9: 409 alreadyQueued is "alreadyQueued" with the open request id from error.message', async () => {
    on('POST', '/api/app/web-requests', () => json(409, { error: { code: 'alreadyQueued', message: 'k-old' } }));
    expect(await queueApi().create(input)).toEqual({ kind: 'alreadyQueued', requestId: 'k-old' });
  });

  it('409 alreadyQueued that names no id (message "alreadyQueued") rejects with the ApiError', async () => {
    on('POST', '/api/app/web-requests', () =>
      json(409, { error: { code: 'alreadyQueued', message: 'alreadyQueued' } }),
    );
    const err = await queueApi()
      .create(input)
      .then(
        () => undefined,
        (e: unknown) => e,
      );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('alreadyQueued');
  });

  it('a refused quote (400 quoteRefused) rejects with the ApiError', async () => {
    on('POST', '/api/app/web-requests', () =>
      json(400, { error: { code: 'quoteRefused', message: 'สกุลเงินนี้ปิดรับ' } }),
    );
    const err = await queueApi()
      .create(input)
      .then(
        () => undefined,
        (e: unknown) => e,
      );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).code).toBe('quoteRefused');
  });
});

describe('QW-2a: QueueApi commands on an open request', () => {
  it('reissueQr posts {branch} to /{id}/qr, keyed, and reads the fresh token; two reissues use two keys', async () => {
    let n = 0;
    on('POST', '/api/app/web-requests/k-old/qr', () => {
      n += 1;
      return json(200, { requestId: 'k-old', requestQrUrl: `/q#tok-${n}`, expiresAt: EXPIRES });
    });
    const api = queueApi();
    expect(await api.reissueQr('b-id-7', 'k-old')).toEqual({ requestId: 'k-old', token: 'tok-1', expiresAt: EXPIRES });
    expect(await api.reissueQr('b-id-7', 'k-old')).toEqual({ requestId: 'k-old', token: 'tok-2', expiresAt: EXPIRES });
    expect(writes()).toHaveLength(2);
    expect(writes()[0]!.body).toEqual({ branch: 'b-id-7' });
    const [a, b] = writes().map((w) => w.headers.get('Idempotency-Key'));
    expect(a).toBeTruthy();
    expect(b).toBeTruthy();
    expect(b).not.toBe(a);
  });

  it('reissueQr: 404 notFound rejects with the ApiError', async () => {
    on('POST', '/api/app/web-requests/k-gone/qr', () => json(404, { error: { code: 'notFound', message: 'notFound' } }));
    const err = await queueApi()
      .reissueQr('b-id-7', 'k-gone')
      .then(
        () => undefined,
        (e: unknown) => e,
      );
    expect(err).toBeInstanceOf(ApiError);
    expect((err as ApiError).status).toBe(404);
  });

  it('U4 wire: cancel posts {branch} to /{id}/cancel, keyed, once; the answer is CANCELLED', async () => {
    on('POST', '/api/app/web-requests/k-1/cancel', () => json(200, { requestId: 'k-1', status: 'CANCELLED' }));
    expect(await queueApi().cancel('b-id-7', 'k-1')).toEqual({ requestId: 'k-1', status: 'CANCELLED' });
    expect(writes()).toHaveLength(1);
    expect(writes()[0]!.body).toEqual({ branch: 'b-id-7' });
    expect(writes()[0]!.headers.get('Idempotency-Key')).toBeTruthy();
  });

  it('U5 wire: emailCopy posts {subject: "webRequest", id, qr} to /api/app/copy-by-email, once', async () => {
    on('POST', '/api/app/copy-by-email', () => json(200, { sent: true }));
    expect(await queueApi().emailCopy('k-1', 'tok-1')).toEqual({ sent: true });
    expect(writes()).toHaveLength(1);
    expect(writes()[0]!.path).toBe('/api/app/copy-by-email');
    expect(writes()[0]!.body).toEqual({ subject: 'webRequest', id: 'k-1', qr: 'tok-1' });
  });
});
