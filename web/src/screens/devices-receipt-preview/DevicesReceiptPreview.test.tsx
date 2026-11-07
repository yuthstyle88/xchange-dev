// OL-6 wave B devices-receipt-page (audit admin B row 054): the slip preview page. It shows the unchanged server slip
// `/devices/receipt?branch=…&ref=…` (api/src/presentation/pages.rs:393, pinned byte for byte by
// api/tests/devices_receipt_route_test.rs) in a frame inside the staff shell, with a title, a way back to /devices and a
// primary print button that prints the frame only. `main.tsx` mounts it inside AppShell (shell-look.test.ts sweeps
// every page entry for that). Written before the fix by claude/94389a6b. Do not edit, delete or
// weaken; add your own elsewhere.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { DevicesReceiptPreview } from './DevicesReceiptPreview';
import { en, th } from './i18n';

interface NodeFs {
  readFileSync(file: string, encoding: 'utf8'): string;
}

let css = '';

beforeAll(async () => {
  const specifier = 'node:fs';
  const fs = (await import(/* @vite-ignore */ specifier)) as NodeFs;
  const here = decodeURIComponent(import.meta.url.replace(/^file:\/\//, '').replace(/[^/]*$/, ''));
  css = fs.readFileSync(here + 'devices-receipt-preview.css', 'utf8');
});

function rule(selector: string): string {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const re = /([^{}]+)\{([^{}]*)\}/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(stripped)) !== null) if (m[1]!.trim() === selector) return m[2]!;
  return '';
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ version: 1, changed: false }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        }),
    ),
  );
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.unstubAllGlobals();
});

function mount(search: string): void {
  dispose = render(() => <DevicesReceiptPreview search={search} />, host);
}

const frame = () => host.querySelector<HTMLIFrameElement>('iframe');
const printButton = () => host.querySelector<HTMLButtonElement>('button[data-action="print"]');

describe('devices receipt preview page', () => {
  it('RP-1 the entry renders the page inside the staff shell with the slip title', async () => {
    host.id = 'app';
    window.history.replaceState(null, '', '/devices/receipt-preview?branch=B1&ref=TX-2');
    await import('./main');
    expect(host.querySelector('.xc-shell.xc-theme')).not.toBeNull();
    expect(host.querySelector('.xc-shell-title')?.textContent).toBe(th.heading);
    expect(host.querySelector('iframe')?.getAttribute('src')).toBe('/devices/receipt?branch=B1&ref=TX-2');
    expect(th.heading).toBe('ตัวอย่างสลิป 80 มม.');
    expect(en.heading).toBe('80 mm slip preview');
    host.replaceChildren();
  });

  it('RP-2 frames the unchanged server slip for the same branch and ref, inside a card', () => {
    mount('?branch=B1&ref=TX-2');
    expect(frame()?.getAttribute('src')).toBe('/devices/receipt?branch=B1&ref=TX-2');
    expect(frame()?.closest('.card')).not.toBeNull();
    expect(frame()?.getAttribute('title')).toBe('TX-2');
  });

  it('RP-3 encodes what it puts in the frame URL', () => {
    mount('?branch=B%201&ref=TX%262');
    expect(frame()?.getAttribute('src')).toBe('/devices/receipt?branch=B%201&ref=TX%262');
  });

  it('RP-4 links back to /devices for the same branch', () => {
    mount('?branch=B1&ref=TX-2');
    const back = host.querySelector<HTMLAnchorElement>('a[data-action="back"]');
    expect(back?.getAttribute('href')).toBe('/devices?branch=B1');
    expect(back?.textContent).toBe(th.back);
    expect(th.back).toBe('← งานอุปกรณ์');
    expect(en.back).toBe('← Devices');
  });

  it('RP-5 with no branch the way back is plain /devices', () => {
    mount('?ref=TX-2');
    expect(host.querySelector('a[data-action="back"]')?.getAttribute('href')).toBe('/devices');
    expect(frame()?.getAttribute('src')).toBe('/devices/receipt?ref=TX-2');
  });

  it('RP-6 the print button is primary, says พิมพ์, and prints the frame, not the page', () => {
    mount('?branch=B1&ref=TX-2');
    const button = printButton()!;
    expect(button.classList.contains('xc-btn')).toBe(true);
    expect(button.classList.contains('xc-btn--primary')).toBe(true);
    expect(button.textContent).toBe(th.print);
    expect(th.print).toBe('พิมพ์');
    expect(en.print).toBe('Print');
    const framePrint = vi.fn();
    Object.defineProperty(frame()!, 'contentWindow', { value: { print: framePrint }, configurable: true });
    const pagePrint = vi.spyOn(window, 'print').mockImplementation(() => {});
    button.click();
    expect(framePrint).toHaveBeenCalledTimes(1);
    expect(pagePrint).not.toHaveBeenCalled();
  });

  it('RP-7 with no ref: no frame, a notice, and no print button', () => {
    mount('?branch=B1');
    expect(frame()).toBeNull();
    expect(printButton()).toBeNull();
    expect(host.querySelector('[data-notice="no-ref"]')?.textContent).toBe(th.noRef);
    expect(th.noRef).toBe('ไม่ระบุเลขรายการ');
    expect(en.noRef).toBe('No transaction reference');
  });

  it('RP-8 the slip card is centred, at most 360px wide, and never wider than a phone screen', () => {
    expect(rule('.xc-receipt-preview')).toMatch(/justify-items:\s*center/);
    const card = rule('.xc-receipt-preview-card');
    expect(card).toMatch(/box-sizing:\s*border-box/);
    expect(card).toMatch(/width:\s*100%/);
    expect(card).toMatch(/max-width:\s*360px/);
  });
});
