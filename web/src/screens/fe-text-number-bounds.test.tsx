import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../shared/i18n';
import {
  makeApprovalRequestItemDto,
  makeRateAlertSettingsData,
  makeReversalPendingRow,
} from '../shared/testing/fixtures';
import { decideRefusal } from './admin/parts/decide';
import { Approvals } from './admin/approvals/Approvals';
import { ReversalsPart } from './admin/reconciliation/reversals/ReversalsPart';
import { AssistantScreen } from './customer/assistant/AssistantScreen';
import { TicketActions } from './desk/queue/actions/TicketActions';
import { RateAlertSettings } from './admin/rate-alert-settings/RateAlertSettings';
import { th as rateAlertTh } from './admin/rate-alert-settings/i18n';

describe('fe-text-number-bounds (#4972)', () => {
  let container: HTMLDivElement;
  let dispose: (() => void) | null = null;

  beforeEach(() => {
    setLang('th');
    container = document.createElement('div');
    document.body.appendChild(container);
  });

  afterEach(() => {
    dispose?.();
    dispose = null;
    container.remove();
    vi.restoreAllMocks();
  });

  // Row 1: Approval/reversal reject reason: maxLength 500, counter n / 500, decideRefusal refuses >500
  it('Row 1: Approval/reversal reject reason enforces maxLength 500, counter n / 500, and decideRefusal refuses over 500 with worded line', async () => {
    // 1. decideRefusal length check
    expect(decideRefusal('REJECTED', 'a'.repeat(501))).toBe('reasonTooLong');
    expect(decideRefusal('APPROVED', 'a'.repeat(501))).toBe('reasonTooLong');
    expect(decideRefusal('REJECTED', 'a'.repeat(500))).toBeNull();
    expect(decideRefusal('REJECTED', '')).toBe('reasonRequired');

    // 2. Approvals screen
    vi.stubGlobal('fetch', async (input: RequestInfo | URL) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      if (url.includes('/api/v3/views/approval_requests')) {
        const item = makeApprovalRequestItemDto({
          id: 'req-1',
          type: 'RATE_PUBLICATION',
          status: 'PENDING',
          requestedBy: 'u-teller',
        });
        return new Response(JSON.stringify({ data: { requests: [item] } }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        });
      }
      return new Response(JSON.stringify({}), { status: 200 });
    });

    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    dispose = render(() => <Approvals branch={branch} period={period} shown={shown} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('input')).not.toBeNull();
    });

    const appInput = container.querySelector('input')!;
    expect(appInput.maxLength).toBe(500);
    expect(container.textContent).toContain('0 / 500');

    appInput.value = 'hello';
    appInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(container.textContent).toContain('5 / 500');

    // 3. ReversalsPart component
    dispose();
    const revRow = makeReversalPendingRow({
      requestId: 'rev-1',
      transactionId: 'tx-1',
      requestedBy: 'u-teller',
    });
    dispose = render(
      () => <ReversalsPart rows={[revRow]} branch="MAIN" onDecided={async () => {}} />,
      container,
    );

    const revInput = container.querySelector<HTMLInputElement>('input[name="reason"]')!;
    expect(revInput).not.toBeNull();
    expect(revInput.maxLength).toBe(500);
    expect(container.textContent).toContain('0 / 500');

    revInput.value = 'invalid amount';
    revInput.dispatchEvent(new Event('input', { bubbles: true }));
    expect(container.textContent).toContain('14 / 500');
  });

  // Row 2: Assistant message: maxLength 500
  it('Row 2: Assistant message enforces maxLength 500', async () => {
    vi.stubGlobal('fetch', async () => new Response(JSON.stringify({}), { status: 200 }));
    dispose = render(() => <AssistantScreen />, container);

    const input = container.querySelector<HTMLInputElement>('.assistant-input');
    expect(input).not.toBeNull();
    expect(input!.maxLength).toBe(500);
  });

  // Row 3: Queue ticket cancel reason: maxLength 200
  it('Row 3: Queue ticket cancel reason enforces maxLength 200', async () => {
    const fakeCommands = {
      issueWalkIn: vi.fn(),
      callNext: vi.fn(),
      ticket: vi.fn(),
      cancel: vi.fn(),
      counter: vi.fn(),
      queue: vi.fn(),
    };
    dispose = render(
      () => (
        <TicketActions
          ticket={{ id: 't-1', displayNumber: 'A001', status: 'WAITING', tellerId: null }}
          me={{ userId: 'u-1', permissions: ['QueueOperate'] }}
          commands={fakeCommands}
          onDone={() => {}}
        />
      ),
      container,
    );

    const cancelBtn = container.querySelector<HTMLButtonElement>('button[name="cancel"]')!;
    expect(cancelBtn).not.toBeNull();
    cancelBtn.click();

    const reasonInput = container.querySelector<HTMLInputElement>('input[name="cancelReason"]');
    expect(reasonInput).not.toBeNull();
    expect(reasonInput!.maxLength).toBe(200);
  });

  // Row 4: Rate-alert settings: cooldown max 720, max active max 100, lifetime max 365
  it('Row 4: Alert cooldown / max active / lifetime enforces bounds with worded lines and sends nothing over max', async () => {
    const sent: Array<{ method: string; path: string; body: unknown }> = [];
    vi.stubGlobal('fetch', async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
      const u = new URL(url, 'http://localhost');
      const method = (init?.method ?? 'GET').toUpperCase();
      let body: unknown = null;
      if (typeof init?.body === 'string') {
        try {
          body = JSON.parse(init.body);
        } catch {
          body = init.body;
        }
      }
      sent.push({ method, path: u.pathname, body });

      if (u.pathname === '/api/v3/views/rate_alert_settings') {
        return new Response(
          JSON.stringify({
            data: makeRateAlertSettingsData({
              defaultCooldownSeconds: 86400,
              maxActivePerCustomer: 10,
              defaultLifetimeDays: 30,
              observePolicy: 'ALL_FRESH',
            }),
          }),
          { status: 200, headers: { 'content-type': 'application/json' } },
        );
      }
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    });

    const [branch] = createSignal('MAIN');
    const [period] = createSignal({ from: '2026-10-23', to: '2026-10-23' });
    const [shown] = createSignal(true);
    dispose = render(() => <RateAlertSettings branch={branch} period={period} shown={shown} />, container);

    await vi.waitFor(() => {
      expect(container.querySelector('form')).not.toBeNull();
    });

    const cooldownInput = container.querySelector<HTMLInputElement>('input[name="defaultCooldownHours"]')!;
    const maxActiveInput = container.querySelector<HTMLInputElement>('input[name="maxActivePerCustomer"]')!;
    const lifetimeInput = container.querySelector<HTMLInputElement>('input[name="defaultLifetimeDays"]')!;
    const saveBtn = container.querySelector<HTMLButtonElement>('button[type="submit"]')!;

    expect(cooldownInput.getAttribute('max')).toBe('720');
    expect(maxActiveInput.getAttribute('max')).toBe('100');
    expect(lifetimeInput.getAttribute('max')).toBe('365');

    // 1. cooldown > 720
    cooldownInput.value = '721';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();
    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="cooldown"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="cooldown"]')?.textContent).toContain(rateAlertTh.errorCooldownMax);
    expect(sent.filter((r) => r.method === 'PUT')).toHaveLength(0);

    // Reset cooldown to valid
    cooldownInput.value = '24';
    cooldownInput.dispatchEvent(new Event('input', { bubbles: true }));

    // 2. max active > 100
    maxActiveInput.value = '101';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();
    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="maxActive"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="maxActive"]')?.textContent).toContain(rateAlertTh.errorMaxActiveMax);
    expect(sent.filter((r) => r.method === 'PUT')).toHaveLength(0);

    // Reset max active to valid
    maxActiveInput.value = '10';
    maxActiveInput.dispatchEvent(new Event('input', { bubbles: true }));

    // 3. lifetime > 365
    lifetimeInput.value = '366';
    lifetimeInput.dispatchEvent(new Event('input', { bubbles: true }));
    saveBtn.click();
    await vi.waitFor(() => {
      expect(container.querySelector('[data-error="lifetime"]')).not.toBeNull();
    });
    expect(container.querySelector('[data-error="lifetime"]')?.textContent).toContain(rateAlertTh.errorLifetimeMax);
    expect(sent.filter((r) => r.method === 'PUT')).toHaveLength(0);
  });
});
