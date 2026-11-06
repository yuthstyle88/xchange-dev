// A plain web visit (no ?branch=, not a paired tablet) chooses its branch inside Step 1 (owner 2026-11-05, "มี3 สเต็ป").
import { describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn, settle } from './load-kiosk';

describe('/kiosk with no ?branch=', () => {
  it('shows the branch control in Step 1 and blocks Review until a branch is chosen', async () => {
    openKiosk('', fakeApi());
    await settle();

    expect(el('branch-choice').hidden).toBe(false);
    expect(el('progress').textContent).toBe('Step 1 of 3 · Select');
    const select = el<HTMLSelectElement>('branch-select');
    expect([...select.options].map((o) => [o.value, o.text])).toEqual([
      ['', 'Choose a branch'],
      ['MAIN', 'Main branch'],
      ['B2', 'Second branch'],
    ]);

    // an amount but no branch: no rates, so Review stays disabled
    (document.querySelector('#quick-amounts button') as HTMLButtonElement).click();
    expect(el<HTMLInputElement>('amount').value).toBe('100');
    expect(document.querySelectorAll('#currency-shortcuts button')).toHaveLength(0);
    expect(el<HTMLButtonElement>('review').disabled).toBe(true);

    select.value = 'MAIN';
    select.dispatchEvent(new Event('change'));
    await ratesDrawn();
    expect(el('branch').textContent).toBe('Main branch');
    expect(el<HTMLSelectElement>('payment-method').value).toBe('Cash'); // preselected
    expect(el<HTMLButtonElement>('review').disabled).toBe(false);
  });

  it('filters the branch list as the customer types', () => {
    const search = el<HTMLInputElement>('branch-search');
    search.value = 'second';
    search.dispatchEvent(new Event('input'));
    expect([...el<HTMLSelectElement>('branch-select').options].map((o) => o.value)).toEqual(['', 'B2']);
  });
});
