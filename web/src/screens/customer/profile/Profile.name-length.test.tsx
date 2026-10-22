// #4418 (FE half of #4397): Customer profile display-name input caps at 100 characters
// and maps 400 nameTooLong to its worded text while keeping the typed value in the input.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient, type Fetch } from '../../../shared/api/client';
import { makeMyProfileData } from '../../../shared/testing/fixtures';
import { setAudience, setLang } from '../../../shared/i18n';
import { ProfileScreen } from './Profile';

interface Write {
  method: string;
  path: string;
  body: Record<string, unknown>;
}

type Reply = { status: number; body: unknown };

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

const PROFILE = makeMyProfileData({
  customerId: 'cus-4418',
  customerNo: 'C-004418',
  displayName: 'สมชาย',
  preferredLanguage: 'th',
  status: 'ACTIVE',
  offeredLanguages: ['th', 'en', 'zh'],
});

function harness(opts: { profileWrite?: Reply } = {}) {
  const writes: Write[] = [];
  const fetchFn: Fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const method = (init?.method ?? 'GET').toUpperCase();
    const u = new URL(input, 'http://localhost');
    if (method === 'GET' && u.pathname === '/api/v3/views/my_profile') {
      return json(200, { version: 1, changed: true, data: PROFILE });
    }
    if (method === 'PUT' && u.pathname === '/api/app/profile') {
      const body = JSON.parse(typeof init?.body === 'string' ? init.body : '{}');
      writes.push({ method, path: u.pathname, body });
      const r = opts.profileWrite ?? {
        status: 200,
        body: {
          customerId: PROFILE.customerId,
          displayName: body.displayName ?? PROFILE.displayName,
          preferredLanguage: body.preferredLanguage ?? PROFILE.preferredLanguage,
        },
      };
      return json(r.status, r.body);
    }
    return json(404, { error: { code: 'notFound', message: 'Not found' } });
  });
  return { api: new ApiClient(fetchFn), writes };
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

function saveButton(): HTMLButtonElement {
  return host.querySelector('button[data-action="save-profile"]') as HTMLButtonElement;
}

function typeInto(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('ProfileScreen displayName length and nameTooLong (#4418)', () => {
  it('sets maxLength === 100 on the display-name input', async () => {
    mount(harness().api);
    await flush();
    expect(nameInput().maxLength).toBe(100);
  });

  it('sends a 100-character Thai name in the save body unchanged', async () => {
    const h = harness();
    mount(h.api);
    await flush();
    const thai100 = 'ก'.repeat(100);
    typeInto(nameInput(), thai100);
    await flush();
    saveButton().click();
    await flush();
    expect(h.writes).toHaveLength(1);
    expect(h.writes[0]?.body).toEqual({ displayName: thai100, preferredLanguage: null });
    expect(host.querySelector('[data-testid="profile-saved"]')).not.toBeNull();
  });

  it('shows the th nameTooLong text on 400 nameTooLong and keeps the typed name in the input', async () => {
    const h = harness({
      profileWrite: {
        status: 400,
        body: { error: { code: 'nameTooLong', message: 'displayName must be at most 100 characters' } },
      },
    });
    mount(h.api);
    await flush();
    const overlong = 'ก'.repeat(101);
    typeInto(nameInput(), overlong);
    await flush();
    saveButton().click();
    await flush();
    expect(host.querySelector('[data-testid="profile-error"]')?.textContent?.trim()).toBe(
      'ชื่อที่แสดงยาวได้ไม่เกิน 100 ตัวอักษร',
    );
    expect(host.querySelector('[data-testid="profile-saved"]')).toBeNull();
    expect(nameInput().value).toBe(overlong);
  });
});
