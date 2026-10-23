import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setAudience, setLang } from '../../../shared/i18n';
import { th as sharedTh } from '../../../shared/i18n/th';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { BookScreen } from './Book';
import { th as screenTh } from './i18n';

const BRANCHES = {
  branches: [
    {
      id: 'XC-B01',
      code: 'XC-B01',
      name: 'สาขาสยามพารากอน',
      timezone: 'Asia/Bangkok',
      bookingEnabled: true,
    },
  ],
};

const SLOTS = {
  data: {
    slots: [
      {
        id: 'slot-101',
        start: '2099-10-15T03:00:00Z',
        end: '2099-10-15T03:30:00Z',
        available: 3,
      },
    ],
  },
};

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('Customer Book (C02) estimate preview error clearing and race ordering (#2193)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  let previewResponder: (body: { amount?: { quantity?: string } }) => Promise<Response>;
  let createResponder: (body: Record<string, unknown>, headers: Headers) => Promise<Response>;
  let bookingWrites: { headers: Headers; rawBody: string }[];

  beforeEach(() => {
    setLang('th');
    setAudience('customer');
    host = document.createElement('div');
    document.body.appendChild(host);
    bookingWrites = [];

    previewResponder = async () =>
      json(200, {
        thb: '3542.00',
        rate: '35.42000000',
        unitSize: '1',
        stale: false,
        sourceDate: '2099-10-15',
      });

    createResponder = async () =>
      json(200, {
        id: 'bk-1',
        status: 'CONFIRMED',
        branchId: 'XC-B01',
        slotId: 'slot-101',
        shopSide: 'BUY',
        currency: 'USD',
        quantity: 100,
      });

    const fetchMock = vi.fn(async (input: string, init?: RequestInit): Promise<Response> => {
      const u = new URL(input, 'http://xc.test');
      if (u.pathname === '/api/public/host') {
        return json(200, { org: 'XC' });
      }
      if (u.pathname.startsWith('/api/public/orgs/XC/branches')) {
        return json(200, BRANCHES);
      }
      if (u.pathname === '/api/v3/views/booking_slots') {
        return json(200, SLOTS);
      }
      if (u.pathname === '/api/v3/compute/planner_preview') {
        const parsed = JSON.parse(String(init?.body ?? '{}')) as {
          amount?: { quantity?: string };
        };
        return previewResponder(parsed);
      }
      if (u.pathname === '/api/bookings' && (init?.method ?? 'GET').toUpperCase() === 'POST') {
        const headers = new Headers(init?.headers);
        const rawBody = String(init?.body ?? '{}');
        bookingWrites.push({ headers, rawBody });
        const parsed = JSON.parse(rawBody) as Record<string, unknown>;
        return createResponder(parsed, headers);
      }
      return json(404, { error: { code: 'notFound', message: u.pathname } });
    });

    vi.stubGlobal('fetch', fetchMock);
    dispose = render(() => <BookScreen fetch={fetchMock} />, host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
    window.history.replaceState({}, '', '/app/book');
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  async function selectBranchAndSlot(): Promise<{
    qtyInput: HTMLInputElement;
    submitBtn: HTMLButtonElement;
  }> {
    await vi.waitFor(() => {
      expect(host.querySelector('option[value="XC-B01"]')).not.toBeNull();
    });
    const branchSel = host.querySelector<HTMLSelectElement>('select[name="branchId"]')!;
    branchSel.value = 'XC-B01';
    branchSel.dispatchEvent(new Event('change', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector('option[value="slot-101"]')).not.toBeNull();
    });
    const slotSel = host.querySelector<HTMLSelectElement>('select[name="slotId"]')!;
    slotSel.value = 'slot-101';
    slotSel.dispatchEvent(new Event('change', { bubbles: true }));

    const qtyInput = host.querySelector<HTMLInputElement>('input[name="quantity"]')!;
    const submitBtn = host.querySelector<HTMLButtonElement>('button[type="submit"]')!;
    return { qtyInput, submitBtn };
  }

  it('Row 1: Preview fails with a network error — shows error.network and clears when a subsequent preview succeeds', async () => {
    const { qtyInput } = await selectBranchAndSlot();

    previewResponder = async () => {
      throw new TypeError('Failed to fetch');
    };
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      const alert = host.querySelector('.alert-error[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.network']);
    });

    previewResponder = async () => json(200, { thb: '3542.00' });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
      expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });
  });

  it('Row 2: Preview fails, then the next preview succeeds — the alert set by the preview is gone', async () => {
    const { qtyInput } = await selectBranchAndSlot();

    previewResponder = async () =>
      json(500, { error: { code: 'serverError', message: 'RAW-preview-500' } });
    qtyInput.value = '50';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      const alert = host.querySelector('.alert-error[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(sharedTh['error.serverError']);
    });

    previewResponder = async () => json(200, { thb: '7084.00' });
    qtyInput.value = '200';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
      const previewEl = host.querySelector('[data-testid="planner-preview"]');
      expect(previewEl).not.toBeNull();
      expect(previewEl!.textContent).toContain(formatMoney('7084.00'));
    });
  });

  it('Row 3: Answers for "10" then "100" arrive in reverse order — the estimate shown is the "100" answer', async () => {
    const { qtyInput } = await selectBranchAndSlot();

    let resolve10!: (res: Response) => void;
    let resolve100!: (res: Response) => void;

    previewResponder = (body) => {
      const q = body.amount?.quantity;
      if (q === '10') {
        return new Promise<Response>((r) => {
          resolve10 = r;
        });
      }
      return new Promise<Response>((r) => {
        resolve100 = r;
      });
    };

    qtyInput.value = '10';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    // "100" answers first
    resolve100(json(200, { thb: '3542.00' }));
    await vi.waitFor(() => {
      const previewEl = host.querySelector('[data-testid="planner-preview"]');
      expect(previewEl).not.toBeNull();
      expect(previewEl!.textContent).toContain(formatMoney('3542.00'));
    });

    // Older "10" answer arrives later and must be dropped
    resolve10(json(200, { thb: '354.20' }));
    await new Promise((r) => setTimeout(r, 20));

    const previewEl = host.querySelector('[data-testid="planner-preview"]');
    expect(previewEl).not.toBeNull();
    expect(previewEl!.textContent).toContain(formatMoney('3542.00'));
    expect(previewEl!.textContent).not.toContain(formatMoney('354.20'));

    // Also verify an older failed request arriving after a newer success does not wipe the estimate or set an alert
    qtyInput.value = '10';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    resolve100(json(200, { thb: '3542.00' }));
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });
    resolve10(json(500, { error: { code: 'serverError', message: 'Stale error' } }));
    await new Promise((r) => setTimeout(r, 20));

    expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
    expect(host.querySelector('[data-testid="planner-preview"]')!.textContent).toContain(
      formatMoney('3542.00'),
    );
  });

  it('Row 4: Box cleared while a request is in flight — no estimate shown', async () => {
    const { qtyInput } = await selectBranchAndSlot();

    let resolveInFlight!: (res: Response) => void;
    previewResponder = () =>
      new Promise<Response>((r) => {
        resolveInFlight = r;
      });

    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    // Clear the box while the preview for "100" is still in flight
    qtyInput.value = '';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    resolveInFlight(json(200, { thb: '3542.00' }));
    await new Promise((r) => setTimeout(r, 20));

    expect(host.querySelector('[data-testid="planner-preview"]')).toBeNull();
  });

  it('Row 5: A create error is still shown until the next submit, while a preview error clears on preview success', async () => {
    const { qtyInput, submitBtn } = await selectBranchAndSlot();

    // First: preview error clears when preview succeeds
    previewResponder = async () =>
      json(500, { error: { code: 'serverError', message: 'Preview error' } });
    qtyInput.value = '50';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).not.toBeNull();
    });

    previewResponder = async () => json(200, { thb: '1771.00' });
    qtyInput.value = '50';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
    });

    // Second: create error stays up across a successful preview until the next submit
    createResponder = async () =>
      json(400, {
        error: { code: 'slotUnavailable', message: 'Slot unavailable' },
      });
    submitBtn.click();

    await vi.waitFor(() => {
      const alert = host.querySelector('.alert-error[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(screenTh['error.slotUnavailable']);
    });

    previewResponder = async () => json(200, { thb: '3542.00' });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      const previewEl = host.querySelector('[data-testid="planner-preview"]');
      expect(previewEl).not.toBeNull();
      expect(previewEl!.textContent).toContain(formatMoney('3542.00'));
    });

    const alertAfterPreview = host.querySelector('.alert-error[role="alert"]');
    expect(alertAfterPreview).not.toBeNull();
    expect(alertAfterPreview!.textContent).toContain(screenTh['error.slotUnavailable']);
  });

  it('Row 6: 500 -> preview 500 -> preview 200 -> submit reuses the same key, and subsequent preview recovery clears its own error', async () => {
    const { qtyInput, submitBtn } = await selectBranchAndSlot();

    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });

    let attempts = 0;
    createResponder = async () => {
      attempts++;
      if (attempts === 1) {
        return json(500, { error: { code: 'internalError', message: 'Server error' } });
      }
      return json(200, { id: 'bk-ok', status: 'CONFIRMED' });
    };

    submitBtn.click();
    await vi.waitFor(() => {
      expect(bookingWrites.length).toBe(1);
      expect(host.querySelector('.alert-error[role="alert"]')).not.toBeNull();
    });

    previewResponder = async () =>
      json(500, { error: { code: 'internalError', message: 'Preview failed' } });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 20));

    previewResponder = async () => json(200, { thb: '3542.00' });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });

    // Submit again with same input -> reuses same idempotency key and clears create error
    submitBtn.click();
    await vi.waitFor(() => {
      expect(bookingWrites.length).toBe(2);
      expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
    });
    expect(bookingWrites[1]!.headers.get('idempotency-key')).toBe(
      bookingWrites[0]!.headers.get('idempotency-key'),
    );

    // After create succeeded, a preview 500 -> preview 200 clears the preview error
    previewResponder = async () =>
      json(500, { error: { code: 'internalError', message: 'Preview failed again' } });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).not.toBeNull();
    });

    previewResponder = async () => json(200, { thb: '3542.00' });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await vi.waitFor(() => {
      expect(host.querySelector('.alert-error[role="alert"]')).toBeNull();
    });
  });

  it('Edge: A preview error and a create error at once — the create error wins and is not cleared by a later preview', async () => {
    const { qtyInput, submitBtn } = await selectBranchAndSlot();

    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    createResponder = async () =>
      json(400, {
        error: { code: 'capacityFull', message: 'Capacity full' },
      });
    submitBtn.click();

    await vi.waitFor(() => {
      const alert = host.querySelector('.alert-error[role="alert"]');
      expect(alert).not.toBeNull();
      expect(alert!.textContent).toContain(screenTh['error.capacityFull']);
    });

    // Preview fails with network error while create error is active -> create error wins
    previewResponder = async () => {
      throw new TypeError('Failed to fetch');
    };
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));
    await new Promise((r) => setTimeout(r, 20));

    let alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(screenTh['error.capacityFull']);
    expect(alert!.textContent).not.toContain(sharedTh['error.network']);

    // Later preview succeeds -> create error is still not cleared
    previewResponder = async () => json(200, { thb: '3542.00' });
    qtyInput.value = '100';
    qtyInput.dispatchEvent(new Event('input', { bubbles: true }));

    await vi.waitFor(() => {
      expect(host.querySelector('[data-testid="planner-preview"]')).not.toBeNull();
    });
    alert = host.querySelector('.alert-error[role="alert"]');
    expect(alert).not.toBeNull();
    expect(alert!.textContent).toContain(screenTh['error.capacityFull']);
  });
});
