// F4 (#564 issuecomment-6624851184): ApiClient keeps a v3 refusal's structured `error.details`
// (design xchange-v3.md §4.3 `V3Error::with_details`), so a screen can word it in the page's language.
// Written before the implementation by the ticket author claude/01TxaRJG. Do not edit, delete or weaken.
import { describe, expect, it } from 'vitest';
import { ApiClient, ApiError, type Fetch } from './client';

function client(status: number, body: unknown): ApiClient {
  const fetchFn: Fetch = async () =>
    new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
  return new ApiClient(fetchFn);
}

async function refusalOf(status: number, body: unknown): Promise<ApiError> {
  try {
    await client(status, body).post('/api/transactions', {});
  } catch (e) {
    expect(e).toBeInstanceOf(ApiError);
    return e as ApiError;
  }
  throw new Error('expected a refusal');
}

const SHORT = {
  lines: [
    { currency: 'USD', place: 'DRAWER', shortBy: '7' },
    { currency: 'EUR', place: 'SAFE', shortBy: '5.5' },
  ],
};

describe('ApiClient: a v3 refusal keeps error.details', () => {
  it('D1: an object error with details exposes them unchanged', async () => {
    const e = await refusalOf(409, {
      error: { code: 'stockInsufficient', message: 'ยอดเงินสดไม่พอ ต้องโอนเงินเข้าก่อน: USD ลิ้นชัก ขาด 7', details: SHORT },
    });
    expect(e.code).toBe('stockInsufficient');
    expect(e.status).toBe(409);
    expect((e as ApiError & { details?: unknown }).details).toEqual(SHORT);
  });

  it('D2: an object error without details has none', async () => {
    const e = await refusalOf(409, { error: { code: 'stockInsufficient', message: 'x' } });
    expect((e as ApiError & { details?: unknown }).details).toBeUndefined();
  });
});
