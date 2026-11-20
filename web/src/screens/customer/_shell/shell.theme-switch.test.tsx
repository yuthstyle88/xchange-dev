// Dark mode for customer pages (design xchange-one-look.md §2.6; owner 2026-11-20 "ไม่มี dark mode"): the customer shell
// carries the same light / dark switch as staff pages, in its header just before the language buttons, on the same store.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CustomerShell } from './Shell';

describe('customer shell light / dark switch', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('th');
    localStorage.clear();
    host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <CustomerShell route="/app" />, host);
  });

  afterEach(() => {
    dispose?.();
    host.remove();
    delete document.documentElement.dataset.theme;
    localStorage.clear();
  });

  it('CTS-1 the header has one switch, right before the first language button', () => {
    const header = host.querySelector('.xc-customer .xc-shell-header')!;
    const switches = header.querySelectorAll(':scope > button.xc-theme-switch');
    expect(switches).toHaveLength(1);
    const next = switches[0]!.nextElementSibling as HTMLElement | null;
    expect(next?.tagName).toBe('BUTTON');
    expect(next?.textContent?.trim()).toBe('TH');
  });

  it('CTS-2 pressing it flips <html data-theme> and saves the choice under the staff key', () => {
    const btn = host.querySelector('button.xc-theme-switch') as HTMLButtonElement;
    const before = document.documentElement.dataset.theme;
    btn.click();
    const after = document.documentElement.dataset.theme;
    expect(after).not.toBe(before);
    expect(localStorage.getItem('xc-theme-mode')).toBe(after);
    expect(btn.getAttribute('aria-pressed')).toBe(after === 'dark' ? 'true' : 'false');
  });
});
