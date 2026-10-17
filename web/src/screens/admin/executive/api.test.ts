// #1909: A01 executive/api.ts fetchOverview over ApiClient.view('executive_overview', { filter }).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '../../../shared/api/client';
import type { ExecutiveOverviewData } from '../../../shared/api/generated/executive_overview';
import { fetchOverview, type OverviewQuery } from './api';

const ENDPOINT = '/api/v3/views/executive_overview';

const SAMPLE_QUERY: OverviewQuery = {
  from: '2026-10-05',
  to: '2026-10-06',
  branches: [],
  channel: '',
  currency: '',
};

const SAMPLE_DATA: ExecutiveOverviewData = {
  period: { from: '2026-10-05', to: '2026-10-06' },
  kpis: [],
  branches: [],
  alerts: [],
  definitions: [],
  buyReversedThb: '0.00',
  sellReversedThb: '0.00',
};

interface RecordedCall {
  url: URL;
  init: RequestInit;
}

function stubViewFetch(
  reply: (call: RecordedCall) => { status: number; statusText?: string; body: string; contentType?: string },
): RecordedCall[] {
  const calls: RecordedCall[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
      const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, 'http://xc.test');
      if (url.pathname !== ENDPOINT) {
        throw new Error(`expected GET ${ENDPOINT}, got ${url.pathname}`);
      }
      const call: RecordedCall = { url, init: init ?? {} };
      calls.push(call);
      const r = reply(call);
      return new Response(r.body, {
        status: r.status,
        statusText: r.statusText ?? '',
        headers: { 'content-type': r.contentType ?? 'application/json' },
      });
    }),
  );
  return calls;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('executive/api fetchOverview (#1909)', () => {
  it('a1: full load with empty branches, channel, and currency sends one GET to /api/v3/views/executive_overview with from and to only', async () => {
    const calls = stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ changed: true, versions: { B1: 1 }, data: SAMPLE_DATA }),
    }));

    await fetchOverview(SAMPLE_QUERY);

    expect(calls).toHaveLength(1);
    const { url, init } = calls[0]!;
    expect(url.pathname).toBe(ENDPOINT);
    expect(init.method ?? 'GET').toBe('GET');
    expect(init.credentials).toBe('same-origin');
    expect(url.searchParams.get('from')).toBe('2026-10-05');
    expect(url.searchParams.get('to')).toBe('2026-10-06');
    expect(url.searchParams.has('branches')).toBe(false);
    expect(url.searchParams.has('channel')).toBe(false);
    expect(url.searchParams.has('currency')).toBe(false);
    expect(url.searchParams.has('versions')).toBe(false);
  });

  it('a2: sends comma-joined branches with empty items dropped in order, plus channel and currency', async () => {
    const calls = stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ changed: true, versions: { B2: 1, B1: 2 }, data: SAMPLE_DATA }),
    }));

    await fetchOverview({
      from: '2026-10-05',
      to: '2026-10-06',
      branches: ['B2', '', 'B1'],
      channel: 'Kiosk',
      currency: 'USD',
    });

    expect(calls).toHaveLength(1);
    const params = calls[0]!.url.searchParams;
    expect(params.get('branches')).toBe('B2,B1');
    expect(params.get('channel')).toBe('Kiosk');
    expect(params.get('currency')).toBe('USD');
  });

  it('a3: poll sends versions JSON for both a populated map and an empty map {}', async () => {
    const calls = stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ changed: false, versions: { B1: 3, B2: 7 } }),
    }));

    await fetchOverview(SAMPLE_QUERY, { B1: 3, B2: 7 });
    await fetchOverview(SAMPLE_QUERY, {});

    expect(calls).toHaveLength(2);
    const firstVersions = calls[0]!.url.searchParams.get('versions');
    expect(firstVersions).not.toBeNull();
    expect(JSON.parse(firstVersions!)).toEqual({ B1: 3, B2: 7 });

    const secondVersions = calls[1]!.url.searchParams.get('versions');
    expect(secondVersions).not.toBeNull();
    expect(JSON.parse(secondVersions!)).toEqual({});
  });

  it('a4: resolves 200 {changed:true, versions, data} with changed, versions, and data', async () => {
    stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ changed: true, versions: { B1: 3 }, data: SAMPLE_DATA }),
    }));

    const ans = await fetchOverview(SAMPLE_QUERY);
    expect(ans).toEqual({
      changed: true,
      versions: { B1: 3 },
      data: SAMPLE_DATA,
    });
  });

  it('a5: resolves 200 {changed:false, versions} with changed:false, versions, and no data', async () => {
    stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ changed: false, versions: { B1: 3 } }),
    }));

    const ans = await fetchOverview(SAMPLE_QUERY, { B1: 3 });
    expect(ans).toEqual({
      changed: false,
      versions: { B1: 3 },
    });
    expect('data' in ans).toBe(false);
    expect(ans.data).toBeUndefined();
  });

  it('b1: rejects 403 outOfScope with ApiError code outOfScope, message, and status 403', async () => {
    stubViewFetch(() => ({
      status: 403,
      body: JSON.stringify({ error: { code: 'outOfScope', message: 'm' } }),
    }));

    const err = await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({
      code: 'outOfScope',
      message: 'm',
      status: 403,
    });
  });

  it('b2: rejects 400 validationFailed with ApiError code, field branches, and status 400', async () => {
    stubViewFetch(() => ({
      status: 400,
      body: JSON.stringify({ error: { code: 'validationFailed', field: 'branches', message: 'm' } }),
    }));

    const err = await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({
      code: 'validationFailed',
      field: 'branches',
      message: 'm',
      status: 400,
    });
  });

  it('b3: preserves error.reason on ApiError when present', async () => {
    stubViewFetch(() => ({
      status: 400,
      body: JSON.stringify({ error: { code: 'x', reason: 'r', message: 'm' } }),
    }));

    const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('x');
    expect(err.reason).toBe('r');
    expect(err.message).toBe('m');
    expect(err.status).toBe(400);
  });

  it('b4: turns string {error:"text"} into ApiError code refused with message text', async () => {
    stubViewFetch(() => ({
      status: 400,
      body: JSON.stringify({ error: 'text' }),
    }));

    const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('refused');
    expect(err.message).toBe('text');
  });

  it('b5: turns non-JSON error body into ApiError code httpError with body text and status 502', async () => {
    stubViewFetch(() => ({
      status: 502,
      body: 'oops',
      contentType: 'text/plain',
    }));

    const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('httpError');
    expect(err.message).toBe('oops');
    expect(err.status).toBe(502);
  });

  it('b6: turns empty error body into ApiError code httpError with statusText and status 500', async () => {
    stubViewFetch(() => ({
      status: 500,
      statusText: 'Internal Server Error',
      body: '',
      contentType: 'text/plain',
    }));

    const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('httpError');
    expect(err.message).toBe('Internal Server Error');
    expect(err.status).toBe(500);
  });

  it('b7: passes roleNotAllowed and roleNotAllowedHere through unmapped (not turned into outOfScope)', async () => {
    for (const code of ['roleNotAllowed', 'roleNotAllowedHere'] as const) {
      stubViewFetch(() => ({
        status: 400,
        body: JSON.stringify({ error: { code, message: 'm' } }),
      }));

      const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
      expect(err).toBeInstanceOf(ApiError);
      expect(err.code).toBe(code);
      expect(err.code).not.toBe('outOfScope');
      expect(err.status).toBe(400);
      expect(err.message).toBe('m');
    }
  });

  it('c1: api.ts contains no raw fetch( and no local refusal( parser', () => {
    const rawFiles = import.meta.glob<string>('./api.ts', {
      query: '?raw',
      import: 'default',
      eager: true,
    });
    const src = rawFiles['./api.ts'] ?? '';
    expect(src.length).toBeGreaterThan(0);
    expect(src).not.toContain('fetch(');
    expect(src).not.toContain('refusal(');
  });

  it('edge: rejects an answer without versions instead of defaulting versions to {}', async () => {
    stubViewFetch(() => ({
      status: 200,
      body: JSON.stringify({ version: 1, changed: true, data: SAMPLE_DATA }),
    }));

    const err = (await fetchOverview(SAMPLE_QUERY).catch((e: unknown) => e)) as ApiError;
    expect(err).toBeInstanceOf(ApiError);
    expect(err.code).toBe('httpError');
    expect(err.status).toBe(200);
  });
});
