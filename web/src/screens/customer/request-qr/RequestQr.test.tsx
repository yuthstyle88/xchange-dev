import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { RequestQr } from './RequestQr';
import { th, en, zh } from './i18n';

describe('RequestQr (QW-5, kiosk-shared-phone §5r F11, row P3)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    setAudience('customer');
    setLang('en');
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    host.remove();
    vi.restoreAllMocks();
  });

  it('P3: shows the "show this QR to staff" text, renders the QR, and never calls fetch', () => {
    const stubFetch = vi.fn();
    const globalFetchSpy = vi.spyOn(globalThis, 'fetch');

    dispose = render(
      () => <RequestQr url="https://example.com/q#test-token-123" fetch={stubFetch} />,
      host,
    );

    // Shows instruction text
    expect(host.textContent).toContain(en.instruction);
    expect(host.textContent).toContain(en.title);

    // Renders the QR code container with SVG
    const qrContainer = host.querySelector('[data-request-qr]');
    expect(qrContainer).not.toBeNull();
    const svg = qrContainer?.querySelector('svg');
    expect(svg).not.toBeNull();

    // Verify fetch was never called
    expect(stubFetch).not.toHaveBeenCalled();
    expect(globalFetchSpy).not.toHaveBeenCalled();
  });

  it('renders Thai and Chinese instructions when language changes', () => {
    setLang('th');
    dispose = render(
      () => <RequestQr url="https://example.com/q#thai-tok" />,
      host,
    );
    expect(host.textContent).toContain(th.instruction);
    expect(host.textContent).toContain(th.title);

    dispose();
    host.innerHTML = '';

    setLang('zh');
    dispose = render(
      () => <RequestQr url="https://example.com/q#zh-tok" />,
      host,
    );
    expect(host.textContent).toContain(zh.instruction);
    expect(host.textContent).toContain(zh.title);
  });

  it('shows invalid text when URL is not a valid request QR URL', () => {
    const stubFetch = vi.fn();
    dispose = render(
      () => <RequestQr url="https://example.com/other-path#some-token" fetch={stubFetch} />,
      host,
    );

    expect(host.textContent).toContain(en.invalid);
    expect(host.querySelector('[data-request-qr]')).toBeNull();
    expect(stubFetch).not.toHaveBeenCalled();
  });
});
