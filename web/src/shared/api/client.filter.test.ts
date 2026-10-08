// V0-C (#475): ApiClient sends a view's filter as query keys
// (docs/design/xchange-v3-web.md §12.1, the `ApiClient` row of §5.2).
// Written before the implementation. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';
import { ApiClient } from './client';

function fakeFetch(body: unknown) {
  const urls: string[] = [];
  const fn = async (url: string): Promise<Response> => {
    urls.push(url);
    return new Response(JSON.stringify(body), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    });
  };
  return { fn, urls };
}

function keysOf(url: string): string[] {
  return [...new URL(url, 'http://xc.test').searchParams.keys()];
}

function paramsOf(url: string): URLSearchParams {
  return new URL(url, 'http://xc.test').searchParams;
}

interface TxFilter {
  q?: string;
  type?: '' | 'BUY' | 'SELL';
  from?: string;
  to?: string;
  minAmount?: number;
}

describe('ApiClient.view with a filter', () => {
  it('sends each filter key after the paging keys and leaves out an empty one', async () => {
    const f = fakeFetch({ version: 1, changed: true, data: {} });
    await new ApiClient(f.fn).view('x', { branch: 'B', filter: { q: 'a', type: '' } });
    expect(f.urls).toEqual(['/api/v3/views/x?branch=B&q=a']);
  });

  it('leaves out undefined and empty values and sends the rest as text', async () => {
    const f = fakeFetch({ version: 4, changed: false });
    const answer = await new ApiClient(f.fn).view<unknown, TxFilter>('admin_transactions', {
      branch: 'XC-B01',
      since: 4,
      limit: 20,
      filter: { from: '2026-10-01', to: undefined, q: 'ส ม', type: '', minAmount: 5 },
    });
    expect(answer).toEqual({ version: 4, changed: false });
    expect(keysOf(f.urls[0]!)).toEqual(['branch', 'since', 'limit', 'from', 'q', 'minAmount']);
    const p = paramsOf(f.urls[0]!);
    expect(p.get('from')).toBe('2026-10-01');
    expect(p.get('q')).toBe('ส ม');
    expect(p.get('minAmount')).toBe('5');
    expect(p.has('to')).toBe(false);
    expect(p.has('type')).toBe(false);
  });

  it('sends a number 0 and the text "0", which are not empty', async () => {
    const f = fakeFetch({ version: 1, changed: true, data: {} });
    await new ApiClient(f.fn).view<unknown, { minAmount?: number; q?: string }>('x', {
      filter: { minAmount: 0, q: '0' },
    });
    expect(f.urls).toEqual(['/api/v3/views/x?minAmount=0&q=0']);
  });

  it('sends no query at all when the filter has nothing to send', async () => {
    const f = fakeFetch({ version: 1, changed: true, data: {} });
    await new ApiClient(f.fn).view<unknown, TxFilter>('x', { filter: { q: '', to: undefined } });
    await new ApiClient(f.fn).view<unknown, TxFilter>('x', { filter: {} });
    expect(f.urls).toEqual(['/api/v3/views/x', '/api/v3/views/x']);
  });

  it('still reads a view without a filter exactly as before', async () => {
    const f = fakeFetch({ version: 2, changed: true, data: { n: 1 } });
    const answer = await new ApiClient(f.fn).view<{ n: number }>('me', { branch: 'MAIN', since: 1 });
    expect(answer).toEqual({ version: 2, changed: true, data: { n: 1 } });
    expect(f.urls).toEqual(['/api/v3/views/me?branch=MAIN&since=1']);
  });

  it('types the filter by the view', () => {
    const client = new ApiClient(fakeFetch({ version: 1, changed: true }).fn);
    const typed = (): unknown =>
      // @ts-expect-error: q is a string in TxFilter
      client.view<unknown, TxFilter>('x', { filter: { q: 1 } });
    expect(typeof typed).toBe('function');
  });
});

describe('ApiClient.page with a filter', () => {
  it('sends the filter after branch, before and limit, leaving out empty values', async () => {
    const f = fakeFetch({ version: 3, changed: true, data: [{ id: 'a' }] });
    const rows = await new ApiClient(f.fn).page<{ id: string }, TxFilter>('admin_transactions', {
      branch: 'MAIN',
      before: 'tx-9',
      limit: 50,
      filter: { q: 'abc', type: '', to: '2026-10-08', from: undefined },
    });
    expect(rows).toEqual([{ id: 'a' }]);
    expect(f.urls).toEqual([
      '/api/v3/views/admin_transactions?branch=MAIN&before=tx-9&limit=50&q=abc&to=2026-10-08',
    ]);
  });

  it('sends only the filter when there is no paging key', async () => {
    const f = fakeFetch({ version: 3, changed: true, data: [] });
    await new ApiClient(f.fn).page<{ id: string }, TxFilter>('admin_transactions', {
      filter: { type: 'SELL' },
    });
    expect(f.urls).toEqual(['/api/v3/views/admin_transactions?type=SELL']);
  });
});
