// F46b up-front tests (docs/design/backdated-keying.md §15.2, §15.3 rows F46-2..F46-4): the form's currency list and
// unit come from the backdate_currencies view, the table the server prices with, never from desk_config. Written
// before the implementation by the sub-controller claude/3b81c9d5. Do not edit, delete or weaken; add tests in a
// file of your own.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { ApiClient } from '../../shared/api/client';
import { setLang } from '../../shared/i18n';
import { makeDeskConfig, makeDeskRate } from '../../shared/testing/fixtures';
import { BackdatedScreen } from './BackdatedScreen';

function setInput(el: HTMLInputElement, value: string): void {
  el.value = value;
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
}

function json(data: unknown): Response {
  return new Response(JSON.stringify(data), { status: 200, headers: { 'content-type': 'application/json' } });
}

/** backdate_currencies answers USD 1, JPY 100, KRW 1000. desk_config answers only USD, as "1.0000". */
function api(seen: string[]): ApiClient {
  return new ApiClient(async (url) => {
    seen.push(url);
    if (url.startsWith('/api/v3/views/backdate_currencies')) {
      return json({
        version: 1,
        changed: true,
        data: {
          currencies: [
            { code: 'JPY', unitSize: '100', minorUnit: 0 },
            { code: 'KRW', unitSize: '1000', minorUnit: 0 },
            { code: 'USD', unitSize: '1', minorUnit: 2 },
          ],
        },
      });
    }
    if (url.startsWith('/api/v3/views/desk_config')) {
      return json({
        version: 1,
        changed: true,
        data: makeDeskConfig({ rates: [makeDeskRate({ code: 'USD', unitSize: '1.0000', buy: '33.53', sell: '33.90' })] }),
      });
    }
    return json({});
  });
}

describe('desk-backdated F46: units from backdate_currencies', () => {
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

  function rateLabel(): string {
    return host.querySelector<HTMLInputElement>('input[name="rate"]')!.closest('label')!.textContent ?? '';
  }

  it('lists every currency of the view and reads no desk_config', async () => {
    const seen: string[] = [];
    dispose = render(() => <BackdatedScreen api={api(seen)} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('datalist option[value="KRW"]')).not.toBeNull();
    });
    expect(host.querySelector('datalist option[value="JPY"]')).not.toBeNull();
    expect(host.querySelector('datalist option[value="USD"]')).not.toBeNull();
    expect(seen.some((u) => u.startsWith('/api/v3/views/desk_config'))).toBe(false);
  });

  it('F46-2: KRW, absent from desk_config, is priced per 1000', async () => {
    dispose = render(() => <BackdatedScreen api={api([])} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('datalist option[value="KRW"]')).not.toBeNull();
    });
    setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'KRW');
    setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '100000');
    setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '0.0250');
    expect(rateLabel()).toContain('ต่อหน่วย 1000 KRW');
    // 100000 × 0.0250 ÷ 1000 = 2.50
    expect(host.querySelector('[data-testid="computed-thb"]')?.textContent).toContain('2.50 THB');
  });

  it('F46-2b: JPY is priced per 100', async () => {
    dispose = render(() => <BackdatedScreen api={api([])} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('datalist option[value="JPY"]')).not.toBeNull();
    });
    setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'JPY');
    setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '10000');
    setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '21.555');
    // 10000 × 21.555 ÷ 100 = 2155.50
    expect(host.querySelector('[data-testid="computed-thb"]')?.textContent).toContain('2155.50 THB');
  });

  it('F46-3: USD reads "ต่อหน่วย 1 USD", never "1.0000"', async () => {
    dispose = render(() => <BackdatedScreen api={api([])} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('datalist option[value="USD"]')).not.toBeNull();
    });
    setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'USD');
    expect(rateLabel()).toContain('ต่อหน่วย 1 USD');
    expect(rateLabel()).not.toContain('1.0000');
  });

  it('F46-4: an unknown code shows no unit and no computed THB', async () => {
    dispose = render(() => <BackdatedScreen api={api([])} />, host);
    await vi.waitFor(() => {
      expect(host.querySelector('datalist option[value="USD"]')).not.toBeNull();
    });
    setInput(host.querySelector<HTMLInputElement>('input[name="currency"]')!, 'XXX');
    setInput(host.querySelector<HTMLInputElement>('input[name="quantity"]')!, '100');
    setInput(host.querySelector<HTMLInputElement>('input[name="rate"]')!, '1');
    expect(rateLabel()).not.toMatch(/ต่อหน่วย\s*\d/);
    const computed = host.querySelector('[data-testid="computed-thb"]')?.textContent ?? '';
    expect(computed).not.toMatch(/\d+\.\d{2} THB/);
    expect(computed).toContain('—');
  });
});
