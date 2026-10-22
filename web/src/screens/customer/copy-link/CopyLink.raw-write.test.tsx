import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { CopyLink } from './CopyLink';
import { en } from './i18n';

describe('CopyLink raw-write migration (#4537)', () => {
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

  it('row 3: 500 response shows loadFailed', async () => {
    const fragment = 'v1.webRequest.req-42.1761234567.token-abc.mac-xyz';
    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'internal', message: 'boom' } }), {
        status: 500,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    dispose = render(() => <CopyLink fragment={fragment} fetch={mockFetch} />, host);

    await vi.waitFor(() => {
      expect(host.textContent).toContain(en.loadFailed);
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(host.querySelector('.xc-copy-link-details')).toBeNull();
  });

  it('row 3: network error shows loadFailed', async () => {
    const fragment = 'v1.webRequest.req-42.1761234567.token-abc.mac-xyz';
    const mockFetch = vi.fn().mockRejectedValue(new TypeError('Failed to fetch'));

    dispose = render(() => <CopyLink fragment={fragment} fetch={mockFetch} />, host);

    await vi.waitFor(() => {
      expect(host.textContent).toContain(en.loadFailed);
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    expect(host.querySelector('.xc-copy-link-details')).toBeNull();
  });

  it('row 5: request goes through ApiClient with credentials: same-origin and x-xchange-request header', async () => {
    const fragment = 'v1.webRequest.req-42.1761234567.token-abc.mac-xyz';
    const mockView = {
      subject: 'webRequest',
      id: 'req-42',
      branch: 'Siam Paragon',
      rows: [{ field: 'currency', value: 'USD' }],
      status: 'WAITING',
      qr: 'https://example.com/q#token-abc',
      expiresAt: '2026-10-23T10:00:00Z',
    };

    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(mockView), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    dispose = render(() => <CopyLink fragment={fragment} fetch={mockFetch} />, host);

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-copy-link-details')).not.toBeNull();
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);
    const [url, init] = mockFetch.mock.calls[0] as [string, RequestInit];
    expect(url).toBe('/api/public/copy-link');
    expect(init.method).toBe('POST');
    expect(init.credentials).toBe('same-origin');
    expect(init.headers).toEqual({
      'x-xchange-request': '1',
      'content-type': 'application/json',
    });
    expect(JSON.parse(String(init.body))).toEqual({ link: fragment });
  });
});
