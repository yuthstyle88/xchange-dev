// #2023 KW-4 Phase 1: shared/notifications idempotency-key retry behaviour (§9 rows 1–5) through HTTP only.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { ApiClient, type Fetch } from '../api/client';
import {
  makeMyNotificationItem,
  makeMyNotificationsData,
} from '../testing/fixtures';
import { createNotifications, type NotificationsModel } from './notifications';

interface WriteCall {
  method: string;
  path: string;
  key: string | null;
  body: string;
}

type StepAnswer =
  | { kind: 'network' }
  | { kind: 'http'; status: number; body: unknown };

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const NOTIFICATIONS_DATA = makeMyNotificationsData({
  items: [
    makeMyNotificationItem({
      id: 'n-1',
      seq: 2,
      locale: 'th',
      title: 'USD rate reached 35.10',
      body: 'Tap to book',
      createdAt: '2026-10-15T08:00:00Z',
      read: false,
    }),
    makeMyNotificationItem({
      id: 'n-2',
      seq: 1,
      locale: 'th',
      title: 'Booking confirmed',
      body: 'Siam Head Office',
      createdAt: '2026-10-15T08:05:00Z',
      read: false,
    }),
  ],
});

function makeHarness(steps: StepAnswer[]) {
  const writes: WriteCall[] = [];
  let stepIdx = 0;

  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');

    if (method === 'GET' && u.pathname === '/api/v3/views/my_notifications') {
      return jsonResponse(200, { version: 1, changed: true, data: NOTIFICATIONS_DATA });
    }

    if (method === 'POST' && u.pathname.startsWith('/api/app/notifications/')) {
      const headers = new Headers(init?.headers);
      const body = typeof init?.body === 'string' ? init.body : '';
      writes.push({
        method,
        path: u.pathname,
        key: headers.get('idempotency-key'),
        body,
      });
      const step = steps[stepIdx++] ?? { kind: 'http', status: 200, body: {} };
      if (step.kind === 'network') {
        throw new TypeError('Failed to fetch');
      }
      return jsonResponse(step.status, step.body);
    }

    return jsonResponse(404, { error: { code: 'notFound', message: 'Not found' } });
  });

  return {
    api: new ApiClient(fetchFn),
    writes,
  };
}

describe('notifications.ts — §9 idempotency-key retry behaviour (#2023 KW-4)', () => {
  let disposers: Array<() => void> = [];

  afterEach(() => {
    disposers.forEach((d) => d());
    disposers = [];
    vi.restoreAllMocks();
  });

  function makeModel(api: ApiClient): NotificationsModel {
    let model!: NotificationsModel;
    createRoot((dispose) => {
      disposers.push(dispose);
      model = createNotifications(api);
    });
    return model;
  }

  it('row 1: submit → network TypeError → submit again with the same input sends 2 requests with the same key and byte-equal body', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: {} },
    ]);

    const s = makeModel(api);
    await s.markOneRead('n-1');
    await s.markOneRead('n-1');

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 2: submit → 500 → submit again sends 2 requests with the same key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 500, body: { error: { code: 'internalError', message: 'Server error' } } },
      { kind: 'http', status: 200, body: {} },
    ]);

    const s = makeModel(api);
    await s.markOneRead('n-1');
    await s.markOneRead('n-1');

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBe(writes[0]!.key);
    expect(writes[1]!.body).toBe(writes[0]!.body);
  });

  it('row 3: submit → 4xx refusal → submit again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 400, body: { error: { code: 'validationFailed', message: 'Invalid id' } } },
      { kind: 'http', status: 200, body: {} },
    ]);

    const s = makeModel(api);
    await s.markOneRead('n-1');
    await s.markOneRead('n-1');

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 4: submit → network error → edit the input → submit sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'network' },
      { kind: 'http', status: 200, body: {} },
    ]);

    const s = makeModel(api);
    await s.markOneRead('n-1');
    await s.markOneRead('n-2');

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).not.toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });

  it('row 5: submit → 2xx → submit the same input again sends a new key', async () => {
    const { api, writes } = makeHarness([
      { kind: 'http', status: 200, body: {} },
      { kind: 'http', status: 200, body: {} },
    ]);

    const s = makeModel(api);
    await s.markOneRead('n-1');
    await s.markOneRead('n-1');

    expect(writes).toHaveLength(2);
    expect(writes[0]!.key).toBeTruthy();
    expect(writes[1]!.key).toBeTruthy();
    expect(writes[1]!.body).toBe(writes[0]!.body);
    expect(writes[1]!.key).not.toBe(writes[0]!.key);
  });
});
