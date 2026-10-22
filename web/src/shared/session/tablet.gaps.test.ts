// BT-F1 follow-up (kiosk-shared-phone §7.3 T-D6, T-D10): two rows the provided tablet.test.ts did not pin, found by
// mutation in the PR 4596 review (claude/studio-claude-1: M3 and M4 survived). Written by the XC FE controller
// claude/e8020026. Do not edit, delete or weaken these tests; add tests in a file of your own.
// - A `paired: true` answer with an empty `name` is not a pairing: `{paired: false}`.
// - An empty `#pair=` is no pairing request: 'none', no request, and the URL is left alone.
import { describe, expect, it, vi } from 'vitest';
import { ApiClient } from '../api/client';
import { pairFromFragment, readTabletMode } from './tablet';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

describe('BT-F1 gaps', () => {
  it('T-D6: paired: true with an empty name ⇒ {paired: false}, me read once', async () => {
    const fetchFn = vi.fn(async () => json(200, { paired: true, tabletId: 'bt-1', branchCode: 'MAIN', name: '' }));
    await expect(readTabletMode(fetchFn)).resolves.toEqual({ paired: false });
    expect(fetchFn).toHaveBeenCalledTimes(1);
  });

  it('T-D10: an empty #pair= ⇒ "none": no request, and the URL is left alone', async () => {
    const fetchFn = vi.fn(async () => json(200, { tabletId: 'bt-1', branchCode: 'MAIN', name: 'x' }));
    for (const hash of ['#pair=', '#pair=&x=1']) {
      const history = { replaceState: vi.fn() };
      const loc = { hash, pathname: '/app/queue', search: '?branch=MAIN' };
      await expect(pairFromFragment(new ApiClient(fetchFn), loc, history)).resolves.toBe('none');
      expect(history.replaceState).not.toHaveBeenCalled();
    }
    expect(fetchFn).not.toHaveBeenCalled();
  });
});
