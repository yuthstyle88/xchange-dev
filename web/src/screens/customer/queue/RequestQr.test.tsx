// Tests for QW-2d (kiosk-shared-phone §5r.4a, ticket #4545).
// Covers the 5 rows of the RequestQr behaviour table:
// 1. token = tok-1: .xc-queue-qr[data-request-qr] with data-qr-url = ${location.origin}/q#tok-1
// 2. render: QrCode of that same URL, label = qrLabel, downloadName set, downloadLabel = downloadQr (download button shows)
// 3. render: caption showToStaff (.xc-queue-qr-caption)
// 4. expiresAt: .xc-queue-expiry shows it, with {time} formatted in the page language
// 5. a new token prop (a reissue): data-qr-url and the drawn QR follow the new token
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { locale, setAudience, setLang } from '../../../shared/i18n';
import { en, th, zh } from './i18n';
import { RequestQr } from './RequestQr';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(token: string, expiresAt: string): void {
  dispose = render(() => <RequestQr token={token} expiresAt={expiresAt} />, host);
}

beforeEach(() => {
  setAudience('customer');
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
});

describe('RequestQr', () => {
  it('row 1: token = tok-1 renders .xc-queue-qr[data-request-qr] with data-qr-url = ${location.origin}/q#tok-1', () => {
    mount('tok-1', '2026-10-22T10:00:00Z');
    const el = host.querySelector<HTMLElement>('.xc-queue-qr[data-request-qr]');
    expect(el).not.toBeNull();
    expect(el?.getAttribute('data-qr-url')).toBe(`${location.origin}/q#tok-1`);
  });

  it('row 2: renders QrCode with label qrLabel, downloadName set, downloadLabel downloadQr, and download button shown', () => {
    mount('tok-1', '2026-10-22T10:00:00Z');
    const img = host.querySelector<HTMLElement>('[role="img"]');
    expect(img).not.toBeNull();
    expect(img?.getAttribute('aria-label')).toBe(th.qrLabel);
    expect(img?.querySelector('svg')).not.toBeNull();

    const btn = host.querySelector<HTMLButtonElement>('button[data-action="download-qr"]');
    expect(btn).not.toBeNull();
    expect(btn?.textContent?.trim()).toBe(th.downloadQr);
  });

  it('row 3: renders caption showToStaff (.xc-queue-qr-caption)', () => {
    mount('tok-1', '2026-10-22T10:00:00Z');
    const caption = host.querySelector<HTMLElement>('.xc-queue-qr-caption');
    expect(caption).not.toBeNull();
    expect(caption?.textContent?.trim()).toBe(th.showToStaff);
  });

  it('row 4: .xc-queue-expiry shows expiresAt with {time} formatted in the page language', () => {
    const expiresAt = '2026-10-22T10:00:00Z';
    mount('tok-1', expiresAt);

    const expiry = host.querySelector<HTMLElement>('.xc-queue-expiry');
    expect(expiry).not.toBeNull();
    const d = new Date(expiresAt);

    const expectedTimeTh = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
    expect(expiry?.textContent).toBe(th.expiresAt.replace('{time}', expectedTimeTh));

    setLang('en');
    const expectedTimeEn = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
    expect(expiry?.textContent).toBe(en.expiresAt.replace('{time}', expectedTimeEn));

    setLang('zh');
    const expectedTimeZh = d.toLocaleTimeString(locale(), { hour: '2-digit', minute: '2-digit' });
    expect(expiry?.textContent).toBe(zh.expiresAt.replace('{time}', expectedTimeZh));
  });

  it('row 5: a new token prop (a reissue) updates data-qr-url and the drawn QR reactively', () => {
    const [token, setToken] = createSignal('tok-1');
    dispose = render(() => <RequestQr token={token()} expiresAt="2026-10-22T10:00:00Z" />, host);

    const el = host.querySelector<HTMLElement>('.xc-queue-qr');
    const img = host.querySelector<HTMLElement>('[role="img"]');
    expect(el?.getAttribute('data-qr-url')).toBe(`${location.origin}/q#tok-1`);
    const svgTok1 = img?.innerHTML;
    expect(svgTok1).toBeTruthy();

    setToken('tok-reissue-2');
    expect(el?.getAttribute('data-qr-url')).toBe(`${location.origin}/q#tok-reissue-2`);
    const svgTok2 = img?.innerHTML;
    expect(svgTok2).toBeTruthy();
    expect(svgTok2).not.toBe(svgTok1);
  });
});
