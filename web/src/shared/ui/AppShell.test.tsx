// V0-A: AppShell (docs/design/xchange-v3-web.md §3.2) — v3 screens show the build they were
// compiled into, from GET /health/ready, in their own footer, in the version line's form (#293):
// `v{version} · {build, 7} · {channel}`, with builtAt beside it.
// Written before the implementation. Do not edit, delete or weaken.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
});

const READY = {
  status: 'ok',
  database: true,
  version: '1.0.0',
  build: '0123456789abcdef0123456789abcdef01234567',
  builtAt: '2026-10-08T01:02:03Z',
  channel: 'staging',
  ui: 'built',
};

function mount() {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(
    () => (
      <AppShell title="เข้าสู่ระบบ">
        <p>body</p>
      </AppShell>
    ),
    host,
  );
  return host;
}

async function settle() {
  for (let i = 0; i < 10; i += 1) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

describe('AppShell', () => {
  it('shows its title, its content, and the build line in its footer', async () => {
    const fetch = vi.fn(async () => new Response(JSON.stringify(READY), { status: 200 }));
    vi.stubGlobal('fetch', fetch);
    const host = mount();
    await settle();
    expect(host.textContent).toContain('เข้าสู่ระบบ');
    expect(host.textContent).toContain('body');
    expect(fetch).toHaveBeenCalled();
    expect(String((fetch.mock.calls[0] as unknown[])[0])).toMatch(/\/health\/ready$/);
    const footer = host.querySelector('footer');
    expect(footer?.textContent).toContain('v1.0.0 · 0123456 · staging');
    expect(footer?.textContent).toContain('2026-10-08T01:02:03Z');
  });

  it('still shows the build line when the server answers 503 with its identity', async () => {
    const body = { ...READY, status: 'degraded', database: false };
    vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify(body), { status: 503 })));
    const host = mount();
    await settle();
    expect(host.querySelector('footer')?.textContent).toContain('v1.0.0 · 0123456 · staging');
  });

  it('shows the screen when /health/ready cannot be read', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => Promise.reject(new TypeError('offline'))));
    const host = mount();
    await settle();
    expect(host.textContent).toContain('เข้าสู่ระบบ');
    expect(host.textContent).toContain('body');
    expect(host.querySelector('footer')?.textContent ?? '').not.toContain('undefined');
  });
});
