// Quick amounts (owner 2026-11-05: "ให้มี 500 1000 2000 3000 แล้ว คีย์", "ถ้ากด อีกให้ เป็น เพิ่ม", "100 500 1000 5000", "มี เครื่องหมาย +"):
// an empty field takes the chip's value, every later tap adds to what the field holds.
import { beforeAll, describe, expect, it } from 'vitest';
import { el, fakeApi, openKiosk, ratesDrawn } from './load-kiosk';

const chip = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('#quick-amounts button')].find((b) => b.textContent === label)!;
const key = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>('#amount-keypad button')].find((b) => b.textContent === label)!;
const amount = () => el<HTMLInputElement>('amount').value;
const estimate = () => el('estimate').textContent;
const clear = () => {
  while (amount() !== '') key('⌫').click();
};

describe('quick amounts', () => {
  beforeAll(async () => {
    openKiosk('?branch=MAIN', fakeApi());
    await ratesDrawn();
  });

  it('shows +100 · +500 · +1,000 · +5,000', () => {
    expect([...document.querySelectorAll('#quick-amounts button')].map((b) => b.textContent)).toEqual([
      '+100',
      '+500',
      '+1,000',
      '+5,000',
    ]);
  });

  it('100 then 100 gives 200, and the estimate follows each tap', () => {
    clear();
    chip('+100').click();
    expect(amount()).toBe('100');
    expect(estimate()).toBe('Estimated amount you receive: 3,353.00 THB');
    chip('+100').click();
    expect(amount()).toBe('200');
    expect(estimate()).toBe('Estimated amount you receive: 6,706.00 THB');
    expect(chip('+100').classList.contains('selected')).toBe(true);
  });

  it('5,000 then 1,000 gives 6,000', () => {
    clear();
    chip('+5,000').click();
    chip('+1,000').click();
    expect(amount()).toBe('6000');
    expect(estimate()).toBe('Estimated amount you receive: 201,180.00 THB');
  });

  it('1,000 then 500 gives 1,500, and the highlight is the chip tapped last', () => {
    clear();
    chip('+1,000').click();
    expect(amount()).toBe('1000');
    expect(estimate()).toBe('Estimated amount you receive: 33,530.00 THB');
    chip('+500').click();
    expect(amount()).toBe('1500');
    expect(estimate()).toBe('Estimated amount you receive: 50,295.00 THB');
    expect(chip('+500').classList.contains('selected')).toBe(true);
    expect(chip('+1,000').classList.contains('selected')).toBe(false);
  });

  it('typing 200 then tapping 500 gives 700', () => {
    clear();
    key('2').click();
    key('0').click();
    key('0').click();
    expect(amount()).toBe('200');
    chip('+500').click();
    expect(amount()).toBe('700');
    expect(estimate()).toBe('Estimated amount you receive: 23,471.00 THB');
  });

  it('the keypad goes on from a chip: typing appends, ⌫ deletes, and clears the highlight', () => {
    clear();
    chip('+1,000').click();
    key('5').click();
    expect(amount()).toBe('10005');
    expect(chip('+1,000').classList.contains('selected')).toBe(false);
    key('⌫').click();
    expect(amount()).toBe('1000');
    key('.').click();
    key('5').click();
    expect(amount()).toBe('1000.5');
  });

  it('the keypad is complete: 1-9, ".", 0, ⌫', () => {
    expect([...document.querySelectorAll('#amount-keypad button')].map((b) => b.textContent)).toEqual([
      '1', '2', '3', '4', '5', '6', '7', '8', '9', '.', '0', '⌫',
    ]);
  });
});
