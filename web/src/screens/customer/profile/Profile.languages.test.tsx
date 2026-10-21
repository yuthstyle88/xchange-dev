// #4165: ProfileScreen language options come from my_profile.offeredLanguages (preserving a stale
// current preferredLanguage so it is not silently changed) and 409 consentTextMissing shows its own
// worded text in th/en/zh instead of the generic wordedError fallback.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyProfileData, type MyProfileData } from '../../../shared/testing/fixtures';
import { setAudience, setLang, t as globalT } from '../../../shared/i18n';
import * as table from './i18n';
import { ProfileScreen } from './Profile';

type Reply = { status: number; body: unknown };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

function harness(opts: { profile?: MyProfileData; profileWrite?: Reply } = {}) {
  const data =
    opts.profile ??
    makeMyProfileData({
      customerId: 'cus-4165',
      customerNo: 'C-004165',
      displayName: 'Somchai',
      preferredLanguage: 'th',
      status: 'ACTIVE',
      offeredLanguages: ['th', 'en'],
    });
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/v3/views/my_profile') {
      return json(200, { version: 1, changed: true, data });
    }
    if (method === 'PUT' && u.pathname === '/api/app/profile') {
      const body = JSON.parse(typeof init?.body === 'string' ? init.body : '{}');
      const r = opts.profileWrite ?? {
        status: 200,
        body: {
          customerId: data.customerId,
          displayName: body.displayName ?? data.displayName,
          preferredLanguage: body.preferredLanguage ?? data.preferredLanguage,
        },
      };
      return json(r.status, r.body);
    }
    return json(404, { error: { code: 'notFound', message: 'Not found' } });
  });
  return { api: new ApiClient(fetchFn), fetchFn };
}

async function flush(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await new Promise((r) => setTimeout(r, 0));
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(api: ApiClient): void {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <ProfileScreen client={api} />, host);
}

function langSelect(): HTMLSelectElement {
  return host.querySelector('select[data-field="preferredLanguage"]') as HTMLSelectElement;
}

function saveButton(): HTMLButtonElement {
  return host.querySelector('button[data-action="save-profile"]') as HTMLButtonElement;
}

beforeEach(() => {
  setAudience('customer');
  setLang('en');
});
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('ProfileScreen languages and consentTextMissing (#4165)', () => {
  it('lists only offeredLanguages from my_profile and omits unoffered languages', async () => {
    const h = harness({
      profile: makeMyProfileData({
        customerId: 'cus-4165',
        customerNo: 'C-004165',
        displayName: 'Somchai',
        preferredLanguage: 'th',
        status: 'ACTIVE',
        offeredLanguages: ['th', 'en'],
      }),
    });
    mount(h.api);
    await flush();
    expect(Array.from(langSelect().options).map((o) => o.value)).toEqual(['th', 'en']);
    expect(langSelect().value).toBe('th');
  });

  it('keeps a stale preferredLanguage selected even when it is no longer in offeredLanguages', async () => {
    const h = harness({
      profile: makeMyProfileData({
        customerId: 'cus-4165',
        customerNo: 'C-004165',
        displayName: 'Somchai',
        preferredLanguage: 'zh',
        status: 'ACTIVE',
        offeredLanguages: ['th', 'en'],
      }),
    });
    mount(h.api);
    await flush();
    expect(Array.from(langSelect().options).map((o) => o.value)).toEqual(['zh', 'th', 'en']);
    expect(langSelect().value).toBe('zh');
    expect(saveButton().disabled).toBe(true);
  });

  it.each(['th', 'en', 'zh'] as const)(
    '409 consentTextMissing on save shows its own worded text in %s instead of the generic wordedError',
    async (l) => {
      setLang(l);
      const expected = table[l].consentTextMissing;
      expect(expected.trim().length).toBeGreaterThan(0);

      const h = harness({
        profileWrite: {
          status: 409,
          body: {
            error: {
              code: 'consentTextMissing',
              field: 'preferredLanguage',
              message: 'Consent text missing',
            },
          },
        },
      });
      mount(h.api);
      await flush();

      langSelect().value = 'en';
      langSelect().dispatchEvent(new Event('change', { bubbles: true }));
      await flush();
      saveButton().click();
      await flush();

      const shown = host.querySelector('[data-testid="profile-error"]')?.textContent?.trim();
      expect(shown).toBe(expected);
      expect(shown).not.toBe(globalT('error.serverError'));
    },
  );
});
