import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CopyLink } from './CopyLink';
import { th, en, zh } from './i18n';

describe('CopyLink (QW-5, kiosk-shared-phone §5r F12, §4 D22, row P4)', () => {
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

  it('happy read: one POST with the fragment; details and QR render; read-only', async () => {
    const fragment = 'v1.webRequest.req-42.1761234567.token-abc.mac-xyz';
    const mockDetails = {
      status: 'WAITING',
      branch: 'B01',
      branchName: 'Siam Paragon',
      displayNumber: 'A042',
      currency: 'USD',
      amount: '500.00',
      rate: '34.50',
    };

    const mockFetch = vi.fn().mockImplementation(async (url: string, init?: RequestInit) => {
      expect(url).toBe('/api/public/copy-link');
      expect(init?.method).toBe('POST');
      expect(init?.headers).toEqual({ 'Content-Type': 'application/json' });
      const parsedBody = JSON.parse(String(init?.body));
      expect(parsedBody).toEqual({ fragment });

      return new Response(JSON.stringify(mockDetails), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    });

    dispose = render(
      () => (
        <CopyLink
          url={`https://example.com/c#${fragment}`}
          fetch={mockFetch}
        />
      ),
      host,
    );

    // Wait for onMount async fetch and UI update
    await vi.waitFor(() => {
      expect(host.querySelector('.xc-copy-link-details')).not.toBeNull();
    });

    // Exactly one POST with the fragment
    expect(mockFetch).toHaveBeenCalledTimes(1);

    // Status and details rendered
    expect(host.textContent).toContain('Waiting');
    expect(host.textContent).toContain('Siam Paragon');
    expect(host.textContent).toContain('A042');
    expect(host.textContent).toContain('USD 500.00');
    expect(host.textContent).toContain('34.50');

    // QR drawn from the fragment
    const qrContainer = host.querySelector('[data-copy-qr]');
    expect(qrContainer).not.toBeNull();
    const svg = qrContainer?.querySelector('svg');
    expect(svg).not.toBeNull();

    // Read-only: no cancel, reissue, or sign-in buttons or links (§4 D22)
    const buttons = host.querySelectorAll('button');
    const links = host.querySelectorAll('a');
    expect(buttons.length).toBe(0);
    expect(links.length).toBe(0);
  });

  it('P4: 410 linkExpired shows the worded expiry text', async () => {
    const fragment = 'v1.webRequest.req-42.1700000000.old-token.mac-xyz';
    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'linkExpired', message: 'Link expired' } }), {
        status: 410,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    dispose = render(
      () => (
        <CopyLink
          fragment={fragment}
          fetch={mockFetch}
        />
      ),
      host,
    );

    await vi.waitFor(() => {
      expect(host.textContent).toContain(en.linkExpired);
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(host.querySelector('.xc-copy-link-details')).toBeNull();
    expect(host.querySelector('[data-copy-qr]')).toBeNull();
  });

  it('P4: worded expiry in Thai and Chinese', async () => {
    const fragment = 'v1.booking.bk-1.1700000000.qr-1.mac-1';

    // Thai
    setLang('th');
    const fetchTh = vi.fn().mockResolvedValue(new Response(null, { status: 410 }));
    dispose = render(
      () => <CopyLink fragment={fragment} fetch={fetchTh} />,
      host,
    );
    await vi.waitFor(() => {
      expect(host.textContent).toContain(th.linkExpired);
    });
    dispose();
    host.innerHTML = '';

    // Chinese
    setLang('zh');
    const fetchZh = vi.fn().mockResolvedValue(new Response(null, { status: 410 }));
    dispose = render(
      () => <CopyLink fragment={fragment} fetch={fetchZh} />,
      host,
    );
    await vi.waitFor(() => {
      expect(host.textContent).toContain(zh.linkExpired);
    });
  });

  it('shows invalid when fragment is missing without calling fetch', () => {
    const mockFetch = vi.fn();
    dispose = render(
      () => <CopyLink fragment="" fetch={mockFetch} />,
      host,
    );

    expect(host.textContent).toContain(en.invalid);
    expect(mockFetch).not.toHaveBeenCalled();
  });
});
