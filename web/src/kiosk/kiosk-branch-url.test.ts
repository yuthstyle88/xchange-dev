// With ?branch= the kiosk is that branch's, as before: no branch control, Step 1 straight away.
import { describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn } from './load-kiosk';

describe('/kiosk?branch=MAIN', () => {
  it('has no branch control and loads MAIN in Step 1', async () => {
    const asked: string[] = [];
    const api = fakeApi();
    openKiosk('?branch=MAIN', (url, init) => {
      asked.push(url);
      return api(url, init);
    });
    await ratesDrawn();

    expect(el('branch-choice').hidden).toBe(true);
    expect(el('progress').textContent).toBe('Step 1 of 3 · Select');
    expect(el('branch').textContent).toBe('Main branch');
    // the URL's branch wins: neither the tablet nor the branch list is asked
    expect(asked.filter((u) => u !== '/api/kiosk')).toEqual([]);
  });
});
