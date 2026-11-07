// F51 (docs/design/backdated-keying.md §16.1): a saved backdated deal cannot be booked twice by a second click.
// - After a 200 save the deal's fields are cleared, the saved number and receipt link stay, and the save button is off
//   until a field changes.
// - A send that failed (5xx or no answer) keeps its Idempotency-Key, so clicking again with the same body is a retry
//   the server replays, not a second deal. The form must not call `reset()` before every send.
// Written before the implementation by the ticket author claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { makeStaffCustomerRow, makeTransactionDto } from '../../shared/testing/fixtures';
import { BackdatedScreen } from './BackdatedScreen';

type Call = { key: string | null; body: unknown };

function setInput(el: HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
}

/** A fake server: `answers` is consumed one per POST; the last one repeats. */
function server(answers: Array<'ok' | 'fail'>): { api: ApiClient; calls: Call[] } {
  const calls: Call[] = [];
  const api = new ApiClient(async (url, init) => {
    if (url.startsWith('/api/v3/views/backdate_currencies')) {
      return json({ version: 1, changed: true, data: { currencies: [{ code: 'USD', unitSize: '1', minorUnit: 2 }] } });
    }
    if (url.startsWith('/api/v3/views/staff_customers')) {
      return json({ version: 1, changed: true, data: [makeStaffCustomerRow({ id: 'c-108', displayName: 'Somchai' })] });
    }
    if (url === '/api/transactions/backdated') {
      calls.push({
        key: new Headers(init?.headers).get('idempotency-key'),
        body: typeof init?.body === 'string' ? JSON.parse(init.body) : null,
      });
      const a = answers[Math.min(calls.length - 1, answers.length - 1)];
      if (a === 'fail') return json({ error: { code: 'internal', message: 'boom' } }, 500);
      return json(makeTransactionDto({ id: `tx-${calls.length}`, status: 'COMPLETED', documentRef: `XC-20261104-00000${calls.length}` }));
    }
    return json({ version: 1, changed: false });
  });
  return { api, calls };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  setLang('th');
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  vi.restoreAllMocks();
});

const q = <T extends Element>(sel: string): T => host.querySelector<T>(sel)!;
const submit = (): HTMLButtonElement => q<HTMLButtonElement>('button[type="submit"]');

async function fill(): Promise<void> {
  setInput(q<HTMLInputElement>('input[name="customerSearch"]'), 'Somchai');
  q<HTMLButtonElement>('button[type="button"]').click();
  await vi.waitFor(() => expect(host.querySelector('[data-customer-option="c-108"]')).not.toBeNull());
  q<HTMLButtonElement>('[data-customer-option="c-108"]').click();
  setInput(q<HTMLSelectElement>('select[name="side"]'), 'BUY');
  setInput(q<HTMLInputElement>('input[name="currency"]'), 'USD');
  setInput(q<HTMLInputElement>('input[name="quantity"]'), '500');
  setInput(q<HTMLInputElement>('input[name="rate"]'), '32.15');
  setInput(q<HTMLInputElement>('input[name="expectedSettleThb"]'), '16075.00');
  setInput(q<HTMLInputElement>('input[name="paidAt"]'), '2026-11-04T13:23');
  setInput(q<HTMLTextAreaElement>('textarea[name="notes"]'), 'shop XC-20261104-000018 · teller Ploy');
}

async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) await new Promise((r) => setTimeout(r, 10));
}

describe('F51: a saved deal is not booked twice', () => {
  it('after a save the deal fields are empty, the number and receipt link stay, and save is off', async () => {
    const { api } = server(['ok']);
    dispose = render(() => <BackdatedScreen api={api} />, host);
    await fill();
    submit().click();
    await vi.waitFor(() => expect(q('[data-testid="document-ref"]')?.textContent).toBe('XC-20261104-000001'));
    await settle();

    for (const name of ['quantity', 'rate', 'expectedSettleThb', 'paidAt', 'customerSearch']) {
      expect(q<HTMLInputElement>(`input[name="${name}"]`).value, `${name} is cleared`).toBe('');
    }
    expect(q<HTMLTextAreaElement>('textarea[name="notes"]').value, 'notes is cleared').toBe('');
    expect(q<HTMLAnchorElement>('[data-testid="receipt-link"]').getAttribute('href')).toBe('/desk/transactions/tx-1');
    expect(submit().disabled, 'save is off until a field changes').toBe(true);
  });

  it('a second click right after a save sends nothing', async () => {
    const { api, calls } = server(['ok']);
    dispose = render(() => <BackdatedScreen api={api} />, host);
    await fill();
    submit().click();
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    await vi.waitFor(() => expect(q('[data-testid="document-ref"]')).not.toBeNull());
    submit().click();
    submit().dispatchEvent(new Event('submit', { bubbles: true }));
    q<HTMLFormElement>('form').dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();
    expect(calls).toHaveLength(1);
  });

  it('changing a field after a save turns save back on', async () => {
    const { api } = server(['ok']);
    dispose = render(() => <BackdatedScreen api={api} />, host);
    await fill();
    submit().click();
    await vi.waitFor(() => expect(q('[data-testid="document-ref"]')).not.toBeNull());
    await settle();
    expect(submit().disabled).toBe(true);
    setInput(q<HTMLInputElement>('input[name="quantity"]'), '200');
    await settle();
    expect(submit().disabled).toBe(false);
  });

  it('a failed send keeps its key: clicking again with the same body is a retry, not a new deal', async () => {
    const { api, calls } = server(['fail', 'ok']);
    dispose = render(() => <BackdatedScreen api={api} />, host);
    await fill();
    submit().click();
    await vi.waitFor(() => expect(calls).toHaveLength(1));
    await vi.waitFor(() => expect(submit().disabled).toBe(false));
    submit().click();
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    expect(calls[1]!.body).toEqual(calls[0]!.body);
    expect(calls[1]!.key).toBe(calls[0]!.key);
  });
});
