// Pricing formula FE T4 (docs/design/blueprint/pricing-formula-web.md §2, §3 T1r–T5r): the live preview's table,
// the per-unit note and the `notChecked` list.
import { For, Show, type JSX } from 'solid-js';
import type { PricingFormulaPreviewOutput } from '../../../shared/api/generated/pricing_formula_preview';
import { failureText, parseSpreadFailure } from './spread-failure';
import * as table from './formula-i18n';
import './formula.css';

export interface FormulaPreviewTableProps {
  result: () => PricingFormulaPreviewOutput | null;
  pending: () => boolean;
  error: () => string | null;
}

const t = table.formulaT;

function formatFailure(failure: string): string {
  const parsed = parseSpreadFailure(failure);
  return parsed ? failureText(parsed) : failure;
}

export function FormulaPreviewTable(props: FormulaPreviewTableProps): JSX.Element {
  return (
    <Show when={props.pending() || props.error() || props.result()}>
      <div class="formula-preview">
        <Show when={props.pending()}>
          <div class="formula-pending">{t('previewPending')}</div>
        </Show>

        <Show when={!props.pending() && props.error()}>
          {(err) => (
            <div class="form-error" role="alert">
              {err()}
            </div>
          )}
        </Show>

        <Show when={!props.pending() && !props.error() && props.result()}>
          {(res) => (
            <Show
              when={!res().parseError}
              fallback={<div class="formula-unit-note">{t('previewFixFirst')}</div>}
            >
              <div class="formula-field-label">{t('previewTitle')}</div>
              <table class="formula-preview-table">
                <thead>
                  <tr>
                    <th>{t('colBranch')}</th>
                    <th>{t('colCurrency')}</th>
                    <th>{t('colUnit')}</th>
                    <th>{t('colTier')}</th>
                    <th>{t('colMode')}</th>
                    <th>{t('colBuy')}</th>
                    <th>{t('colSell')}</th>
                    <th>{t('colResult')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={res().rows}>
                    {(row) => {
                      const rowKey = `${row.branch}:${row.currency}:${
                        row.tier !== null && row.tier !== undefined ? row.tier : 'single'
                      }`;
                      const isFailing = Boolean(row.failure);
                      return (
                        <tr
                          data-row={rowKey}
                          class={isFailing ? 'formula-row--fail' : undefined}
                        >
                          <td>{row.branch}</td>
                          <td>{row.currency}</td>
                          <td>
                            {t('unitPer', {
                              unit: row.unitSize,
                              currency: row.currency,
                            })}
                          </td>
                          <td>
                            {row.tier !== null && row.tier !== undefined
                              ? String(row.tier)
                              : t('dash')}
                          </td>
                          <td>
                            {row.mode?.toUpperCase() === 'FOLLOW'
                              ? t('modeFollow')
                              : t('modeAuto')}
                          </td>
                          <td>
                            {row.buy !== null && row.buy !== undefined && row.buy !== ''
                              ? row.buy
                              : t('dash')}
                          </td>
                          <td>
                            {row.sell !== null && row.sell !== undefined && row.sell !== ''
                              ? row.sell
                              : t('dash')}
                          </td>
                          <td>
                            {isFailing ? formatFailure(row.failure!) : t('previewOk')}
                          </td>
                        </tr>
                      );
                    }}
                  </For>
                </tbody>
              </table>

              <Show when={res().rows && res().rows.length > 0}>
                <div class="formula-unit-note">{t('unitNote')}</div>
              </Show>

              <Show when={res().notChecked && res().notChecked.length > 0}>
                <div class="formula-field-label">{t('notCheckedTitle')}</div>
                <ul
                  class="formula-not-checked"
                  data-testid="formula-preview-not-checked"
                >
                  <For each={res().notChecked}>
                    {(nc) => <li>{`${nc.branch} ${nc.currency}`}</li>}
                  </For>
                </ul>
              </Show>
            </Show>
          )}
        </Show>
      </div>
    </Show>
  );
}
