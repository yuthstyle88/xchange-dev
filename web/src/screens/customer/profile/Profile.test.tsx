// C07 up-front tests (feat/fe-customer skeleton): ProfileScreen reads my_profile, saves changes with
// PUT /api/app/profile and records marketing consent with POST /api/app/consents (design
// customer-identity-consent.md §4.3, §4.4, K5). The implementer makes these pass and must not edit them.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyConsent, makeMyProfileData } from '../../../shared/testing/fixtures';
import { setLang } from '../../../shared/i18n';
import { ProfileScreen } from './Profile';

interface Write {
  method: string;
  path: string;
  key: string | null;
  body: Record<string, unknown>;
}

type Reply = { status: number; body: unknown } | 'network';

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const PROFILE = makeMyProfileData({
  customerId: 'cus-1',
  customerNo: 'C-000123',
  displayName: 'Somchai',
  preferredLanguage: 'th',
  status: 'ACTIVE',
  offeredLanguages: ['th', 'en', 'zh'],
  consents: [
    makeMyConsent({ purpose: 'MARKETING', channel: 'EMAIL', granted: true, policyVersion: 2, lang: 'th', effectiveAt: '2026-10-01T00:00:00Z' }),
    makeMyConsent({ purpose: 'MARKETING', channel: 'SMS', granted: false, policyVersion: 2, lang: 'th', effectiveAt: '2026-10-01T00:00:00Z' }),
    makeMyConsent({ purpose: 'TERMS', channel: null, granted: true, policyVersion: 3, lang: 'th', effectiveAt: '2026-09-01T00:00:00Z' }),
  ],
});

function harness(opts: { profile?: Reply; profileWrite?: Reply; consentWrite?: Reply } = {}) {
  const writes: Write[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/v3/views/my_profile') {
      const r = opts.profile ?? { status: 200, body: { version: 1, changed: true, data: PROFILE } };
      if (r === 'network') throw new TypeError('Failed to fetch');
      return json(r.status, r.body);
    }
    if ((method === 'PUT' && u.pathname === '/api/app/profile') || (method === 'POST' && u.pathname === '/api/app/consents')) {
      const body = JSON.parse(typeof init?.body === 'string' ? init.body : '{}');
      writes.push({ method, path: u.pathname, key: new Headers(init?.headers).get('idempotency-key'), body });
      const isProfile = u.pathname === '/api/app/profile';
      const r = (isProfile ? opts.profileWrite : opts.consentWrite) ?? {
        status: 200,
        body: isProfile
          ? { customerId: 'cus-1', displayName: body.displayName ?? PROFILE.displayName, preferredLanguage: body.preferredLanguage ?? PROFILE.preferredLanguage }
          : { purpose: body.purpose, channel: body.channel, granted: body.granted, policyVersion: 2, effectiveAt: '2026-10-19T00:00:00Z' },
      };
      if (r === 'network') throw new TypeError('Failed to fetch');
      return json(r.status, r.body);
    }
    return json(404, { error: { code: 'notFound', message: 'Not found' } });
  });
  return { api: new ApiClient(fetchFn), writes, fetchFn };
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

function nameInput(): HTMLInputElement {
  return host.querySelector('input[data-field="displayName"]') as HTMLInputElement;
}
function langSelect(): HTMLSelectElement {
  return host.querySelector('select[data-field="preferredLanguage"]') as HTMLSelectElement;
}
function saveButton(): HTMLButtonElement {
  return host.querySelector('button[data-action="save-profile"]') as HTMLButtonElement;
}
function consentBox(channel: string): HTMLInputElement {
  return host.querySelector(`input[data-consent="${channel}"]`) as HTMLInputElement;
}
function typeInto(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('ProfileScreen (C07)', () => {
  it('shows the customer number, the display name and the language from my_profile', async () => {
    mount(harness().api);
    await flush();
    expect(host.textContent).toContain('C-000123');
    expect(nameInput().value).toBe('Somchai');
    expect(langSelect().value).toBe('th');
    expect(Array.from(langSelect().options).map((o) => o.value)).toEqual(['th', 'en', 'zh']);
  });

  it('save is disabled until something changes, and while the name is blank', async () => {
    mount(harness().api);
    await flush();
    expect(saveButton().disabled).toBe(true);
    typeInto(nameInput(), '   ');
    await flush();
    expect(saveButton().disabled).toBe(true);
  });

  it('saving a changed name sends PUT /api/app/profile with the new name, null for the unchanged language, and an Idempotency-Key', async () => {
    const h = harness();
    mount(h.api);
    await flush();
    typeInto(nameInput(), 'Somchai J.');
    await flush();
    saveButton().click();
    await flush();
    expect(h.writes).toHaveLength(1);
    expect(h.writes[0]).toMatchObject({ method: 'PUT', path: '/api/app/profile', body: { displayName: 'Somchai J.', preferredLanguage: null } });
    expect(h.writes[0].key).toBeTruthy();
    expect(host.querySelector('[data-testid="profile-saved"]')).not.toBeNull();
    expect(saveButton().disabled).toBe(true);
  });

  it('saving a changed language sends null for the unchanged name', async () => {
    const h = harness();
    mount(h.api);
    await flush();
    langSelect().value = 'zh';
    langSelect().dispatchEvent(new Event('change', { bubbles: true }));
    await flush();
    saveButton().click();
    await flush();
    expect(h.writes[0]?.body).toEqual({ displayName: null, preferredLanguage: 'zh' });
  });

  it('a refused save shows the worded error and keeps the edit', async () => {
    const h = harness({ profileWrite: { status: 422, body: { error: { code: 'validation', field: 'displayName', message: 'too long' } } } });
    mount(h.api);
    await flush();
    typeInto(nameInput(), 'X'.repeat(10));
    await flush();
    saveButton().click();
    await flush();
    expect(host.querySelector('[data-testid="profile-error"]')?.textContent?.trim()).toBeTruthy();
    expect(host.querySelector('[data-testid="profile-saved"]')).toBeNull();
    expect(nameInput().value).toBe('X'.repeat(10));
  });

  it('each marketing channel checkbox shows the newest MARKETING row for it; no row means unchecked', async () => {
    mount(harness().api);
    await flush();
    expect(consentBox('EMAIL').checked).toBe(true);
    expect(consentBox('SMS').checked).toBe(false);
    expect(consentBox('PUSH').checked).toBe(false);
  });

  it('ticking a channel posts MARKETING consent for it in the current language, with an Idempotency-Key', async () => {
    const h = harness();
    mount(h.api);
    await flush();
    consentBox('PUSH').click();
    await flush();
    expect(h.writes).toHaveLength(1);
    expect(h.writes[0]).toMatchObject({
      method: 'POST',
      path: '/api/app/consents',
      body: { purpose: 'MARKETING', channel: 'PUSH', granted: true, lang: 'en' },
    });
    expect(h.writes[0].key).toBeTruthy();
    expect(consentBox('PUSH').checked).toBe(true);
  });

  it('unticking withdraws; a failed consent write puts the box back and shows the error', async () => {
    const h = harness({ consentWrite: { status: 500, body: { error: { code: 'internal', message: 'boom' } } } });
    mount(h.api);
    await flush();
    consentBox('EMAIL').click();
    await flush();
    expect(h.writes[0]?.body).toMatchObject({ purpose: 'MARKETING', channel: 'EMAIL', granted: false });
    expect(consentBox('EMAIL').checked).toBe(true);
    expect(host.querySelector('[data-testid="profile-error"]')?.textContent?.trim()).toBeTruthy();
  });

  it('shows the accepted TERMS version and offers no TERMS toggle', async () => {
    mount(harness().api);
    await flush();
    expect(host.querySelector('[data-testid="terms-version"]')?.textContent).toContain('3');
    expect(host.querySelector('input[data-consent="TERMS"]')).toBeNull();
  });

  it('a 401 from my_profile shows the sign-in prompt and no form', async () => {
    mount(harness({ profile: { status: 401, body: { error: { code: 'unauthenticated', message: 'no' } } } }).api);
    await flush();
    const signIn = host.querySelector('[data-testid="profile-signin"] a') as HTMLAnchorElement | null;
    expect(signIn?.getAttribute('href')).toBe('/app/sign-in');
    expect(nameInput()).toBeNull();
  });

  it('a failed load shows the error and a retry that reads my_profile again', async () => {
    let calls = 0;
    const ok = harness();
    const fetchFn: Fetch = async (input, init) => {
      if (new URL(input, 'http://localhost').pathname === '/api/v3/views/my_profile' && calls++ === 0) {
        throw new TypeError('Failed to fetch');
      }
      return ok.fetchFn(input, init);
    };
    mount(new ApiClient(fetchFn));
    await flush();
    const retry = host.querySelector('button[data-action="retry"]') as HTMLButtonElement | null;
    expect(retry).not.toBeNull();
    retry!.click();
    await flush();
    expect(nameInput().value).toBe('Somchai');
  });
});
