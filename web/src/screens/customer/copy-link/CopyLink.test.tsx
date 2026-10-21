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

  it('happy read: one POST with {link}; branch, rows in order and QR render; read-only', async () => {
    const fragment = 'v1.webRequest.req-42.1761234567.token-abc.mac-xyz';
    const mockView = {
      subject: 'webRequest',
      id: 'req-42',
      branch: 'Siam Paragon',
      rows: [
        { field: 'currency', value: 'USD' },
        { field: 'amount', value: '500.00' },
        { field: 'rate', value: '34.50' },
        { field: 'expiresAt', value: '2026-10-23T10:00:00Z' },
      ],
      status: 'WAITING',
      qr: 'https://example.com/q#token-abc',
      expiresAt: '2026-10-23T10:00:00Z',
    };

    const mockFetch = vi.fn().mockImplementation(async (url: string, init?: RequestInit) => {
      expect(url).toBe('/api/public/copy-link');
      expect(init?.method).toBe('POST');
      expect(init?.headers).toEqual({ 'Content-Type': 'application/json' });
      const parsedBody = JSON.parse(String(init?.body));
      expect(parsedBody).toEqual({ link: fragment });

      return new Response(JSON.stringify(mockView), {
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

    // Exactly one POST with {link: fragment}
    expect(mockFetch).toHaveBeenCalledTimes(1);

    // Status rendered
    expect(host.textContent).toContain('Waiting');

    // Branch rendered
    expect(host.textContent).toContain('Branch');
    expect(host.textContent).toContain('Siam Paragon');

    // Rows rendered in order with labels
    const rows = host.querySelectorAll('.xc-copy-link-row');
    expect(rows.length).toBe(5); // 1 branch + 4 rows
    expect(rows[0]?.textContent).toContain('BranchSiam Paragon');
    expect(rows[1]?.textContent).toContain('CurrencyUSD');
    expect(rows[2]?.textContent).toContain('Amount500.00');
    expect(rows[3]?.textContent).toContain('Exchange Rate34.50');
    expect(rows[4]?.textContent).toContain('Expires At2026-10-23T10:00:00Z');

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

  it('renders booking rows with bookingNo, date, slot and unknown field fallbacks', async () => {
    const fragment = 'v1.booking.bk-1.1761234567.qr-1.mac-1';
    const mockView = {
      subject: 'booking',
      id: 'bk-1',
      branch: 'Asok',
      rows: [
        { field: 'bookingNo', value: 'BK-1001' },
        { field: 'date', value: '2026-10-25' },
        { field: 'slot', value: '10:00 - 10:30' },
        { field: 'currency', value: 'JPY' },
        { field: 'amount', value: '50,000' },
        { field: 'customExtra', value: 'VIP' },
      ],
      status: 'CONFIRMED',
      qr: 'qr-1',
      expiresAt: '2026-10-25T10:30:00Z',
    };

    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(mockView), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    dispose = render(
      () => <CopyLink fragment={fragment} fetch={mockFetch} />,
      host,
    );

    await vi.waitFor(() => {
      expect(host.querySelector('.xc-copy-link-details')).not.toBeNull();
    });

    expect(host.textContent).toContain('Confirmed');
    expect(host.textContent).toContain('Booking Number');
    expect(host.textContent).toContain('BK-1001');
    expect(host.textContent).toContain('Time Slot');
    expect(host.textContent).toContain('10:00 - 10:30');
    // Unknown field shows its raw name
    expect(host.textContent).toContain('customExtra');
    expect(host.textContent).toContain('VIP');
  });

  it('P4: 410 linkExpired shows the worded expiry text', async () => {
    const fragment = 'v1.webRequest.req-42.1700000000.old-token.mac-xyz';
    const mockFetch = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: { code: 'linkExpired', message: 'ลิงก์หมดอายุหรือไม่ถูกต้อง' } }), {
        status: 410,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    dispose = render(
      () => <CopyLink fragment={fragment} fetch={mockFetch} />,
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
