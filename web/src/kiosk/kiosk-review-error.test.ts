// No browser validation bubble (it speaks the browser's language, not the screen's) and no phone keyboard over the
// kiosk keypad; an empty amount gets the kiosk's own worded error in the selected language.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn } from './load-kiosk';

describe('Review with no amount', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', fakeApi());
    await ratesDrawn();
  });

  it('the form never asks the browser to validate', () => {
    expect(el<HTMLFormElement>('form').noValidate).toBe(true);
    expect(el<HTMLInputElement>('amount').inputMode).toBe('none');
  });

  it('shows the translated error, in the language on screen', () => {
    const lang = el<HTMLSelectElement>('lang');
    lang.value = 'th';
    lang.dispatchEvent(new Event('change'));
    el<HTMLSelectElement>('payment-method').value = 'Cash';
    el('payment-method').dispatchEvent(new Event('change'));
    expect(el<HTMLInputElement>('amount').value).toBe('');
    expect(el<HTMLButtonElement>('review').disabled).toBe(false);

    el<HTMLButtonElement>('review').click();
    expect(el('error').textContent).toBe('กรุณากรอกจำนวนเงินและสกุลเงินให้ถูกต้อง');
    expect(el('progress').textContent).toContain('1');
  });
});
