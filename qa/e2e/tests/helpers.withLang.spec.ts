import { expect, test, type Page } from '@playwright/test';
import { type Lang, users, withLang } from './helpers';

test('withLang reloads on same-URL and different-hash goto', async () => {
  const reloads: Array<{ url: string; lang: string }> = [];
  let url = new URL('/#booking-slots', users.baseUrl).href;
  let stored = '';
  const fakePage = {
    url: () => url,
    addInitScript: async () => {},
    evaluate: async (_fn: unknown, l: Lang) => {
      stored = l;
    },
    goto: async (next: string) => {
      url = new URL(next, users.baseUrl).href;
      return null;
    },
    reload: async () => {
      reloads.push({ url, lang: stored });
      return null;
    },
  } as unknown as Page;

  await withLang(fakePage, 'en');
  await fakePage.goto('/#booking-slots');
  expect(reloads).toEqual([{ url: new URL('/#booking-slots', users.baseUrl).href, lang: 'en' }]);

  url = new URL('/#overview', users.baseUrl).href;
  await withLang(fakePage, 'th');
  await fakePage.goto('/#booking-slots');
  expect(reloads).toEqual([
    { url: new URL('/#booking-slots', users.baseUrl).href, lang: 'en' },
    { url: new URL('/#booking-slots', users.baseUrl).href, lang: 'th' },
  ]);
});
