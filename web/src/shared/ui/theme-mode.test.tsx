import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const KEY = 'xc-theme-mode';

async function load() {
  vi.resetModules();
  return import('./theme-mode');
}

describe('theme mode (design §2.6)', () => {
  beforeEach(() => {
    localStorage.clear();
    delete document.documentElement.dataset.theme;
  });
  afterEach(() => vi.unstubAllGlobals());

  it('a saved choice wins over the device setting', async () => {
    localStorage.setItem(KEY, 'light');
    vi.stubGlobal('matchMedia', () => ({ matches: true }));
    const m = await load();
    expect(m.themeMode()).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('with no saved choice it follows the device setting', async () => {
    vi.stubGlobal('matchMedia', () => ({ matches: true }));
    const m = await load();
    expect(m.themeMode()).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('a junk saved value is ignored', async () => {
    localStorage.setItem(KEY, 'purple');
    vi.stubGlobal('matchMedia', () => ({ matches: false }));
    const m = await load();
    expect(m.themeMode()).toBe('light');
  });

  it('toggling flips the page and survives a reload', async () => {
    vi.stubGlobal('matchMedia', () => ({ matches: false }));
    let m = await load();
    m.toggleThemeMode();
    expect(document.documentElement.dataset.theme).toBe('dark');
    expect(localStorage.getItem(KEY)).toBe('dark');
    delete document.documentElement.dataset.theme;
    m = await load();
    expect(m.themeMode()).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('the switch button flips the mode and reports it with aria-pressed', async () => {
    vi.stubGlobal('matchMedia', () => ({ matches: false }));
    await load();
    const { ThemeSwitch } = await import('./ThemeSwitch');
    const host = document.createElement('div');
    document.body.append(host);
    const dispose = render(() => <ThemeSwitch />, host);
    const btn = host.querySelector('button.xc-theme-switch') as HTMLButtonElement;
    expect(btn.getAttribute('aria-pressed')).toBe('false');
    btn.click();
    expect(btn.getAttribute('aria-pressed')).toBe('true');
    expect(document.documentElement.dataset.theme).toBe('dark');
    dispose();
    host.remove();
  });
});
