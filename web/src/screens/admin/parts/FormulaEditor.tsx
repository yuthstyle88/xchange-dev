// Pricing formula FE T3 (docs/design/blueprint/pricing-formula-web.md §2, §3 E1–E4): the two formula texts, the
// inputs a formula may name, and the parse error under its field.
import { createUniqueId, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import type { FormulaTextsDto } from '../../../shared/api/generated/pricing_formula';
import type { FormulaParseErrorDto } from '../../../shared/api/generated/pricing_formula_preview';
import * as table from './formula-i18n';
import './formula.css';

export interface FormulaEditorProps {
  buy: () => string;
  sell: () => string;
  onBuy: (v: string) => void;
  onSell: (v: string) => void;
  /** `PricingFormulaData.inputs` names, in order. */
  inputs: () => readonly string[];
  defaults: () => FormulaTextsDto;
  parseError: () => FormulaParseErrorDto | null;
  disabled: () => boolean;
}

const t = screenT('admin.formula', table);

function formatError(err: FormulaParseErrorDto): string {
  const codeKey = `code.${err.code}` as keyof typeof table.th;
  const prefix = Object.prototype.hasOwnProperty.call(table.th, codeKey)
    ? t(codeKey)
    : err.code;
  return `${prefix}: ${err.message}`;
}

export function FormulaEditor(props: FormulaEditorProps): JSX.Element {
  const buyId = createUniqueId();
  const sellId = createUniqueId();
  return (
    <div class="formula-editor">
      <div class="formula-field">
        <label class="formula-field-label" for={buyId}>
          {t('buyFormulaLabel')}
        </label>
        <textarea
          id={buyId}
          class="formula-textarea"
          data-field="buyFormula"
          maxlength={500}
          value={props.buy()}
          disabled={props.disabled()}
          onInput={(e) => props.onBuy(e.currentTarget.value)}
        />
        <Show when={props.parseError()?.field === 'buyFormula'}>
          <div class="form-error" role="alert" data-field-error="buyFormula">
            {formatError(props.parseError()!)}
          </div>
        </Show>
      </div>

      <div class="formula-field">
        <label class="formula-field-label" for={sellId}>
          {t('sellFormulaLabel')}
        </label>
        <textarea
          id={sellId}
          class="formula-textarea"
          data-field="sellFormula"
          maxlength={500}
          value={props.sell()}
          disabled={props.disabled()}
          onInput={(e) => props.onSell(e.currentTarget.value)}
        />
        <Show when={props.parseError()?.field === 'sellFormula'}>
          <div class="form-error" role="alert" data-field-error="sellFormula">
            {formatError(props.parseError()!)}
          </div>
        </Show>
      </div>

      <div class="formula-help">{t('syntaxHelp')}</div>

      <div class="formula-field-label">{t('inputsTitle')}</div>
      <ul class="formula-inputs">
        <For each={props.inputs()}>
          {(name) => {
            const key = `input.${name}` as keyof typeof table.th;
            const hasMeaning = Object.prototype.hasOwnProperty.call(table.th, key);
            return (
              <li class="formula-input">
                <code>{name}</code>
                {hasMeaning ? ` ${t(key)}` : null}
              </li>
            );
          }}
        </For>
      </ul>

      <div class="formula-actions">
        <button
          type="button"
          data-action="formula-use-default"
          disabled={props.disabled()}
          onClick={() => {
            const defs = props.defaults();
            props.onBuy(defs.buyFormula);
            props.onSell(defs.sellFormula);
          }}
        >
          {t('useDefault')}
        </button>
      </div>
    </div>
  );
}
