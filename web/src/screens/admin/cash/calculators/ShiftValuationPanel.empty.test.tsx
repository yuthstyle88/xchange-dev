// §9 row 7 of #511 (implementer's test): a shift whose `currencies` is empty renders no rate
// input; submitting sends `rates: {}` and shows the answer's result with zero table rows.
import { render } from 'solid-js/web';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiClient } from '../../../../shared/api/client';
import { setLang } from '../../../../shared/i18n';
import { ShiftValuationPanel } from './ShiftValuationPanel';

let dispose: (() => void) | undefined;
let root: HTMLElement | undefined;

afterEach(() => {
  dispose?.();
  root?.remove();
  dispose = undefined;
  root = undefined;
});

const settle = async (): Promise<void> => {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
};

describe('V2-cash-calculators: a shift with no currency (§9 row 7)', () => {
  it('renders no rate input, sends rates: {} and shows the result with zero rows', async () => {
    setLang('th');
    const result = { rows: [], current: '0.00', opening: '0.00', difference: '0.00', movements: '0.00', adjusted: '0.00' };
    const act = vi.fn(async (_kind: string, _name: string, body: Record<string, unknown>) =>
      body.rates === undefined ? { currencies: [] } : { currencies: [], result },
    );
    const api = { act } as unknown as ApiClient;
    root = document.createElement('div');
    document.body.appendChild(root);
    dispose = render(() => <ShiftValuationPanel api={api} branch="B2" shiftId="S-0000" />, root);
    await settle();

    expect(root.querySelectorAll('input[type="number"]').length, 'no rate input').toBe(0);

    const dateInput = root.querySelector<HTMLInputElement>('input[type="date"]')!;
    dateInput.value = '2026-10-06';
    dateInput.dispatchEvent(new Event('input', { bubbles: true }));
    root.querySelector('form')!.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await settle();

    expect(act).toHaveBeenCalledTimes(2);
    expect(act.mock.calls[1]![2]).toEqual({ branch: 'B2', shiftId: 'S-0000', rates: {} });
    const live = root.querySelector('[aria-live="polite"]')!;
    expect(live.querySelector('table'), 'the result table is shown').not.toBeNull();
    expect(live.querySelectorAll('tbody tr').length, 'zero rows').toBe(0);
    expect(live.textContent).toContain('ผู้ใช้กรอกเอง');
  });
});
