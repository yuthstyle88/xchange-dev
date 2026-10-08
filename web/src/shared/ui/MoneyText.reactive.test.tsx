// #449: MoneyText must follow its props after the first render (docs/design/xchange-v3-web.md W7, W10 point 5).
// The digits stay the server's, verbatim; no arithmetic. MoneyText has no labels yet, so a language
// switch must leave its text as it is.
// Written before the fix. Do not edit, delete or weaken.
import { afterEach, describe, expect, it } from 'vitest';
import { batch, createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../i18n';
import { MoneyText } from './MoneyText';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  setLang('th');
});

function mount(value: string, currency: string) {
  const [v, setV] = createSignal(value);
  const [c, setC] = createSignal(currency);
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <MoneyText value={v()} currency={c()} />, host);
  return { host, setV, setC, text: () => host.textContent ?? '' };
}

describe('MoneyText follows its props', () => {
  it('updates the shown amount when the value changes', () => {
    const m = mount('1234.50', 'THB');
    expect(m.text()).toBe('1,234.50 THB');
    m.setV('99.00');
    expect(m.text()).toBe('99.00 THB');
    m.setV('1000000');
    expect(m.text()).toBe('1,000,000 THB');
  });

  it('keeps every digit of a big amount that arrives after the first render', () => {
    const m = mount('0.00', 'THB');
    m.setV('90071992547409.93');
    expect(m.text()).toBe('90,071,992,547,409.93 THB');
    expect(m.text()).not.toContain('409.94');
    m.setV('-1234.50');
    expect(m.text()).toBe('-1,234.50 THB');
  });

  it('updates the currency when it changes', () => {
    const m = mount('35.1000', 'USD');
    expect(m.text()).toBe('35.1000 USD');
    m.setC('JPY');
    expect(m.text()).toBe('35.1000 JPY');
  });

  it('updates value and currency changed in one batch', () => {
    const m = mount('10.00', 'USD');
    batch(() => {
      m.setV('2500.75');
      m.setC('EUR');
    });
    expect(m.text()).toBe('2,500.75 EUR');
  });

  it('switches between a decimal and a non-decimal value', () => {
    const m = mount('abc', 'USD');
    expect(m.text()).toBe('abc USD');
    m.setV('1234.5');
    expect(m.text()).toBe('1,234.5 USD');
    m.setV('1e3');
    expect(m.text()).toBe('1e3 USD');
    expect(m.text()).not.toContain('NaN');
  });

  it('keeps the digits when the language switches', () => {
    setLang('th');
    const m = mount('90071992547409.93', 'THB');
    expect(m.text()).toBe('90,071,992,547,409.93 THB');
    setLang('en');
    expect(m.text()).toBe('90,071,992,547,409.93 THB');
    m.setV('7.10');
    expect(m.text()).toBe('7.10 THB');
    setLang('th');
    expect(m.text()).toBe('7.10 THB');
  });
});
