import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../i18n';
import { sharedSession } from '../session/session';
import { meOf } from '../../test-support/me';
import { AppShell } from './AppShell';

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

const OWNER = meOf({
  id: 'owner',
  role: 'OWNER',
  branch: 'MAIN',
  name: 'Somchai',
  branchName: 'สำนักงานใหญ่',
  branches: [],
});

function stubNetwork() {
  const mockFetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.toString() : input.url;
    if (url.includes('/health/ready')) {
      return new Response(JSON.stringify(READY), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      });
    }
    if (url.includes('/api/v3/views/me')) {
      return new Response(
        JSON.stringify({
          version: 3,
          changed: true,
          data: OWNER,
        }),
        {
          status: 200,
          headers: { 'content-type': 'application/json' },
        },
      );
    }
    return new Response(JSON.stringify({}), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    });
  });
  vi.stubGlobal('fetch', mockFetch);
  return mockFetch;
}

describe('AppShell mobile menu accessible name (OL-1R-c, #6500)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    stubNetwork();
    setAudience('staff');
    setLang('th');
    try {
      window.localStorage.clear();
    } catch {
      // storage blocked
    }
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    try {
      sharedSession().dispose();
    } catch {
      // ignore
    }
    setAudience('staff');
    setLang('th');
    host.remove();
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
  });

  function mount() {
    dispose = render(
      () => (
        <AppShell title="ทดสอบ">
          <p>เนื้อหา</p>
        </AppShell>
      ),
      host,
    );
    const button = host.querySelector<HTMLButtonElement>('.xc-shell-menu-btn');
    expect(button, 'the menu button exists in staff session').not.toBeNull();
    return button!;
  }

  it('staff session, th: the menu button accessible name is เมนู', () => {
    setAudience('staff');
    setLang('th');
    const button = mount();
    expect(button.getAttribute('aria-label')).toBe('เมนู');
  });

  it('en: the menu button accessible name is Menu', () => {
    setAudience('staff');
    setLang('en');
    const button = mount();
    expect(button.getAttribute('aria-label')).toBe('Menu');
  });

  it('zh: the menu button accessible name is 菜单', () => {
    setAudience('customer');
    setLang('zh');
    const button = mount();
    expect(button.getAttribute('aria-label')).toBe('菜单');
  });

  it('menu opened: the name stays the same, and aria-expanded flips', () => {
    setAudience('staff');
    setLang('th');
    const button = mount();

    expect(button.getAttribute('aria-label')).toBe('เมนู');
    expect(button.getAttribute('aria-expanded')).toBe('false');

    button.click();
    expect(button.getAttribute('aria-label')).toBe('เมนู');
    expect(button.getAttribute('aria-expanded')).toBe('true');

    button.click();
    expect(button.getAttribute('aria-label')).toBe('เมนู');
    expect(button.getAttribute('aria-expanded')).toBe('false');
  });
});
