import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { ApiError, type ApiClient, type ViewParams } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { th as sharedTh } from '../../shared/i18n/th';
import { en as sharedEn } from '../../shared/i18n/en';
import {
  makeKbankReference,
  makeKioskCountry,
  makeKioskCurrencies,
  makeKioskCurrency,
  makeRateFreshnessDto,
  makeViewAnswer,
  type KioskCurrencies,
  type KioskCurrency,
  type ViewAnswer,
} from '../../shared/testing/fixtures';
import { KioskCurrenciesScreen } from './KioskCurrenciesScreen';

const USD = makeKioskCurrency({
  code: 'USD',
  name: 'ดอลลาร์สหรัฐ',
  nameEn: 'US Dollar',
  countries: [makeKioskCountry({ code: 'US', en: 'United States', th: 'สหรัฐอเมริกา' })],
  buy: '33.2500',
  sell: '33.5000',
  unitSize: '1',
  updated: '2026-10-06T05:00:01.123Z',
  partnerName: 'SuperRich',
  partnerCost: '32.9000',
  enabled: true,
  kbank: [],
});

function answer(currencies?: KioskCurrency[]) {
  return makeViewAnswer(
    makeKioskCurrencies({
      currencies: currencies ?? [USD],
      kbank: makeKbankReference({ source: 'kbank', publishedAt: '2026-10-06T05:00:00Z', round: 1 }),
      freshness: makeRateFreshnessDto({ stale: false, sourceDate: null, publishedAt: null }),
    }),
    {
      version: 1,
      changed: true,
    },
  );
}

interface FakeClient {
  api: Pick<ApiClient, 'view' | 'post'>;
  views: Array<{ name: string; q?: ViewParams }>;
  posts: Array<{ path: string; body: unknown }>;
}

function createFakeClient(
  viewHandler: (name: string, q?: ViewParams) => Promise<ViewAnswer<KioskCurrencies>>,
  postHandler: (path: string, body: unknown) => Promise<unknown> = async () => ({ saved: true }),
): FakeClient {
  const views: FakeClient['views'] = [];
  const posts: FakeClient['posts'] = [];
  const api: Pick<ApiClient, 'view' | 'post'> = {
    view<T>(name: string, q?: ViewParams): Promise<ViewAnswer<T>> {
      views.push({ name, q: q ? { ...q } : undefined });
      return viewHandler(name, q) as any;
    },
    post<B, R>(path: string, body: B): Promise<R> {
      posts.push({ path, body });
      return postHandler(path, body) as Promise<R>;
    },
  };
  return { api, views, posts };
}

describe('screens/kiosk-currencies wordedError (WE-36 #1785)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setLang('th');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    setLang('th');
  });

  const pageStatuses = (): string[] => {
    return [...host.querySelectorAll('[role="status"]')]
      .filter((el) => !el.closest('form'))
      .map((el) => (el.textContent ?? '').trim());
  };

  const cardStatuses = (): string[] => {
    return [...host.querySelectorAll('form [role="status"]')].map((el) => (el.textContent ?? '').trim());
  };

  describe('fetchCurrencies initial / branch load catch', () => {
    it('(1) network TypeError shows error.network in TH and EN', async () => {
      const client = createFakeClient(async () => {
        throw new TypeError('Failed to fetch');
      });
      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(sharedTh['error.network']);
      });

      // EN
      setLang('en');
      // Re-trigger via re-render
      dispose();
      host.innerHTML = '';
      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code shows error.serverError', async () => {
      const client = createFakeClient(async () => {
        throw new ApiError({
          code: 'roleNotAllowedHere',
          message: 'Internal server error message',
          status: 500,
        });
      });
      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(sharedTh['error.serverError']);
      });

      setLang('en');
      dispose();
      host.innerHTML = '';
      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(sharedEn['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code and server message shows that message', async () => {
      const serverMessage = 'Custom server refusal for currencies view';
      const client = createFakeClient(async () => {
        throw new ApiError({
          code: 'unkeyedCurrenciesRefusal',
          message: serverMessage,
          status: 400,
        });
      });
      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(serverMessage);
      });
    });

    it('after branch switch, a failed read does not leave the previous branch data showing', async () => {
      const [branch, setBranch] = createSignal('B1');
      const client = createFakeClient(async (_name, q) => {
        if (q?.branch === 'B1') {
          return answer([USD]);
        }
        throw new TypeError('Failed to fetch B2');
      });

      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch={branch()} />, host);

      // Verify B1 loaded USD
      await vi.waitFor(() => {
        expect(host.textContent).toContain('USD');
      });

      // Switch to B2 which fails
      setBranch('B2');

      await vi.waitFor(() => {
        expect(pageStatuses()).toContain(sharedTh['error.network']);
      });
      // Previous branch's currencies must be cleared
      expect(host.textContent).not.toContain('USD · ดอลลาร์สหรัฐ');
    });
  });

  describe('handleToggle configure catch', () => {
    it('(1) network TypeError on toggle shows error.network in TH and EN', async () => {
      const client = createFakeClient(
        async () => answer([USD]),
        async () => {
          throw new TypeError('Failed to fetch toggle');
        },
      );

      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);
      await vi.waitFor(() => expect(host.textContent).toContain('USD'));

      const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
      expect(checkbox).not.toBeNull();
      checkbox!.click();

      await vi.waitFor(() => {
        expect(cardStatuses()).toContain(sharedTh['error.network']);
      });

      // Switch language to EN
      setLang('en');
      checkbox!.click();

      await vi.waitFor(() => {
        expect(cardStatuses()).toContain(sharedEn['error.network']);
      });
    });

    it('(2) 500 refusal with keyed non-server code on toggle shows error.serverError', async () => {
      const client = createFakeClient(
        async () => answer([USD]),
        async () => {
          throw new ApiError({
            code: 'roleNotAllowedHere',
            message: 'Internal server toggle failure',
            status: 500,
          });
        },
      );

      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);
      await vi.waitFor(() => expect(host.textContent).toContain('USD'));

      const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
      expect(checkbox).not.toBeNull();
      checkbox!.click();

      await vi.waitFor(() => {
        expect(cardStatuses()).toContain(sharedTh['error.serverError']);
      });
    });

    it('(3) refusal with unkeyed code on toggle shows server message', async () => {
      const serverMessage = 'Currency toggling temporarily locked by head office';
      const client = createFakeClient(
        async () => answer([USD]),
        async () => {
          throw new ApiError({
            code: 'unkeyedToggleRefusal',
            message: serverMessage,
            status: 400,
          });
        },
      );

      dispose = render(() => <KioskCurrenciesScreen api={client.api} branch="B1" />, host);
      await vi.waitFor(() => expect(host.textContent).toContain('USD'));

      const checkbox = host.querySelector<HTMLInputElement>('input[type="checkbox"]');
      expect(checkbox).not.toBeNull();
      checkbox!.click();

      await vi.waitFor(() => {
        expect(cardStatuses()).toContain(serverMessage);
      });
    });
  });
});
