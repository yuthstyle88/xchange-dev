// Pricing formula FE T3 tests (docs/design/blueprint/pricing-formula-web.md §3 E1–E4).
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import type { FormulaTextsDto } from '../../../shared/api/generated/pricing_formula';
import type { FormulaParseErrorDto } from '../../../shared/api/generated/pricing_formula_preview';
import { th } from './formula-i18n';
import { FormulaEditor } from './FormulaEditor';

describe('FormulaEditor (E1–E4)', () => {
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
    setLang('th');
  });

  it('E1: renders two textareas with length limit, labels, help text, and input meanings or names', () => {
    let buyValue = 'ref_buy * 1.01';
    let sellValue = 'ref_sell * 0.99';
    const [buy, setBuy] = createSignal(buyValue);
    const [sell, setSell] = createSignal(sellValue);
    const inputs = () => ['ref_buy', 'ref_sell', 'custom_var'];
    const defaults = (): FormulaTextsDto => ({ buyFormula: 'ref_buy', sellFormula: 'ref_sell' });
    const parseError = (): FormulaParseErrorDto | null => null;
    const disabled = () => false;

    dispose = render(
      () => (
        <FormulaEditor
          buy={buy}
          sell={sell}
          onBuy={(v) => {
            buyValue = v;
            setBuy(v);
          }}
          onSell={(v) => {
            sellValue = v;
            setSell(v);
          }}
          inputs={inputs}
          defaults={defaults}
          parseError={parseError}
          disabled={disabled}
        />
      ),
      host,
    );

    const buyTextarea = host.querySelector<HTMLTextAreaElement>('textarea[data-field="buyFormula"]');
    const sellTextarea = host.querySelector<HTMLTextAreaElement>('textarea[data-field="sellFormula"]');
    expect(buyTextarea).not.toBeNull();
    expect(sellTextarea).not.toBeNull();
    expect(buyTextarea!.getAttribute('maxlength')).toBe('500');
    expect(sellTextarea!.getAttribute('maxlength')).toBe('500');
    expect(buyTextarea!.value).toBe(buyValue);
    expect(sellTextarea!.value).toBe(sellValue);

    const labels = host.querySelectorAll<HTMLLabelElement>('label.formula-field-label');
    expect(labels.length).toBeGreaterThanOrEqual(2);
    expect(labels[0].textContent).toBe(th.buyFormulaLabel);
    expect(labels[1].textContent).toBe(th.sellFormulaLabel);

    expect(host.querySelector('.formula-help')?.textContent).toBe(th.syntaxHelp);
    expect(host.textContent).toContain(th.inputsTitle);

    const inputItems = host.querySelectorAll<HTMLLIElement>('ul.formula-inputs li.formula-input');
    expect(inputItems.length).toBe(3);

    expect(inputItems[0].querySelector('code')?.textContent).toBe('ref_buy');
    expect(inputItems[0].textContent).toContain(th['input.ref_buy']);

    expect(inputItems[1].querySelector('code')?.textContent).toBe('ref_sell');
    expect(inputItems[1].textContent).toContain(th['input.ref_sell']);

    // An input with no text key shows its name only
    expect(inputItems[2].querySelector('code')?.textContent).toBe('custom_var');
    expect(inputItems[2].textContent?.trim()).toBe('custom_var');

    // onBuy / onSell called on input event
    buyTextarea!.value = 'ref_buy * 1.05';
    buyTextarea!.dispatchEvent(new Event('input', { bubbles: true }));
    expect(buyValue).toBe('ref_buy * 1.05');

    sellTextarea!.value = 'ref_sell * 0.95';
    sellTextarea!.dispatchEvent(new Event('input', { bubbles: true }));
    expect(sellValue).toBe('ref_sell * 0.95');
  });

  it('E2: pressing default button calls onBuy and onSell with defaults', () => {
    let capturedBuy = '';
    let capturedSell = '';
    const defaults = (): FormulaTextsDto => ({
      buyFormula: 'defaults_buy_formula',
      sellFormula: 'defaults_sell_formula',
    });

    dispose = render(
      () => (
        <FormulaEditor
          buy={() => ''}
          sell={() => ''}
          onBuy={(v) => {
            capturedBuy = v;
          }}
          onSell={(v) => {
            capturedSell = v;
          }}
          inputs={() => []}
          defaults={defaults}
          parseError={() => null}
          disabled={() => false}
        />
      ),
      host,
    );

    const defaultBtn = host.querySelector<HTMLButtonElement>('button[data-action="formula-use-default"]');
    expect(defaultBtn).not.toBeNull();
    expect(defaultBtn!.textContent).toBe(th.useDefault);

    defaultBtn!.click();
    expect(capturedBuy).toBe('defaults_buy_formula');
    expect(capturedSell).toBe('defaults_sell_formula');
  });

  it('E3: parseError renders code text and message under the failing field only; null renders no error', () => {
    const [parseError, setParseError] = createSignal<FormulaParseErrorDto | null>(null);

    dispose = render(
      () => (
        <FormulaEditor
          buy={() => ''}
          sell={() => ''}
          onBuy={() => {}}
          onSell={() => {}}
          inputs={() => []}
          defaults={() => ({ buyFormula: '', sellFormula: '' })}
          parseError={parseError}
          disabled={() => false}
        />
      ),
      host,
    );

    expect(host.querySelector('.form-error')).toBeNull();

    // Error on sellFormula
    setParseError({
      field: 'sellFormula',
      code: 'syntax',
      message: 'วงเล็บเปิดไม่ครบคู่',
    });

    expect(host.querySelector('[data-field-error="buyFormula"]')).toBeNull();
    const sellErr = host.querySelector<HTMLDivElement>('[data-field-error="sellFormula"]');
    expect(sellErr).not.toBeNull();
    expect(sellErr!.getAttribute('role')).toBe('alert');
    expect(sellErr!.classList.contains('form-error')).toBe(true);
    expect(sellErr!.textContent).toBe(`${th['code.syntax']}: วงเล็บเปิดไม่ครบคู่`);

    // Error on buyFormula
    setParseError({
      field: 'buyFormula',
      code: 'divideByZero',
      message: 'division by 0',
    });

    expect(host.querySelector('[data-field-error="sellFormula"]')).toBeNull();
    const buyErr = host.querySelector<HTMLDivElement>('[data-field-error="buyFormula"]');
    expect(buyErr).not.toBeNull();
    expect(buyErr!.getAttribute('role')).toBe('alert');
    expect(buyErr!.classList.contains('form-error')).toBe(true);
    expect(buyErr!.textContent).toBe(`${th['code.divideByZero']}: division by 0`);

    // Cleared to null
    setParseError(null);
    expect(host.querySelector('.form-error')).toBeNull();
  });

  it('E4: disabled disables both fields and the default button', () => {
    const [disabled, setDisabled] = createSignal(true);

    dispose = render(
      () => (
        <FormulaEditor
          buy={() => 'b'}
          sell={() => 's'}
          onBuy={() => {}}
          onSell={() => {}}
          inputs={() => []}
          defaults={() => ({ buyFormula: 'def_b', sellFormula: 'def_s' })}
          parseError={() => null}
          disabled={disabled}
        />
      ),
      host,
    );

    const buyTextarea = host.querySelector<HTMLTextAreaElement>('textarea[data-field="buyFormula"]')!;
    const sellTextarea = host.querySelector<HTMLTextAreaElement>('textarea[data-field="sellFormula"]')!;
    const defaultBtn = host.querySelector<HTMLButtonElement>('button[data-action="formula-use-default"]')!;

    expect(buyTextarea.disabled).toBe(true);
    expect(sellTextarea.disabled).toBe(true);
    expect(defaultBtn.disabled).toBe(true);

    setDisabled(false);
    expect(buyTextarea.disabled).toBe(false);
    expect(sellTextarea.disabled).toBe(false);
    expect(defaultBtn.disabled).toBe(false);
  });
});
