import { createSignal, For, Show, type JSX } from 'solid-js';
import type { DenominationRow } from '../../api/generated/denominations';
import { lang, screenT } from '../../i18n';
import { formatAmount, formatFaceValue } from './face-value';
import * as table from './i18n';
import './currency-count.css';

const t = screenT('currencyCount', table);

function getCurrencyName(code: string): string {
  try {
    return new Intl.DisplayNames([lang()], { type: 'currency' }).of(code) ?? code;
  } catch {
    return code;
  }
}

export interface CurrencyCountProps {
  denominations: DenominationRow[];
  countable?: string[];
  locked: string[];
  expected?: Record<string, string>;
  quantities: Record<number, string>;
  setQuantity: (id: number, value: string) => void;
  lineErrors: Record<number, string>;
}

export function CurrencyCount(props: CurrencyCountProps): JSX.Element {
  const [extra, setExtra] = createSignal<string[]>([]);
  const [search, setSearch] = createSignal<string>('');

  const added = () => [...props.locked, ...extra().filter((c) => !props.locked.includes(c))];

  const countableCurrencies = () => {
    const allowed = props.countable ? new Set(props.countable) : null;
    const codes = new Set<string>();
    for (const d of props.denominations) {
      if (!allowed || allowed.has(d.currency)) codes.add(d.currency);
    }
    return [...codes].sort();
  };

  const searchMatches = () => {
    const term = search().trim().toLowerCase();
    if (term === '') return [];
    const current = new Set(added());
    return countableCurrencies().filter((code) => {
      if (current.has(code)) return false;
      return (
        code.toLowerCase().includes(term) ||
        getCurrencyName(code).toLowerCase().includes(term)
      );
    });
  };

  const addCurrency = (code: string) => {
    setExtra((prev) => (prev.includes(code) ? prev : [...prev, code]));
    setSearch('');
  };

  const removeCurrency = (code: string) => {
    setExtra((prev) => prev.filter((c) => c !== code));
    for (const d of props.denominations) {
      if (d.currency === code) props.setQuantity(d.id, '');
    }
  };

  const onSearchKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      const first = searchMatches()[0];
      if (first) addCurrency(first);
    }
  };

  return (
    <div class="xc-currency-count">
      <div class="xc-currency-count-search">
        <label class="xc-currency-count-search-label">{t('searchLabel')}</label>
        <input
          data-testid="currency-search"
          class="xc-currency-count-search-input"
          type="text"
          placeholder={t('searchPlaceholder')}
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
          onChange={(e) => setSearch(e.currentTarget.value)}
          onKeyDown={onSearchKeyDown}
        />
        <Show when={search().trim() !== '' && searchMatches().length === 0}>
          <p data-testid="currency-notfound" class="xc-currency-count-notfound">
            {t('searchNotFound')}
          </p>
        </Show>
        <Show when={searchMatches().length > 0}>
          <div class="xc-currency-count-options">
            <For each={searchMatches()}>
              {(code) => (
                <button
                  type="button"
                  data-testid="currency-option"
                  data-currency={code}
                  class="xc-currency-count-option"
                  onClick={() => addCurrency(code)}
                >
                  + {code} {getCurrencyName(code)}
                </button>
              )}
            </For>
          </div>
        </Show>
      </div>

      <div class="xc-currency-count-groups">
        <For each={added()}>
          {(code) => (
            <div
              data-testid="currency-group"
              data-currency={code}
              class="xc-currency-count-group"
            >
              <div class="xc-currency-count-group-head">
                <span class="xc-currency-count-group-title">{code}</span>
                <Show when={props.expected?.[code]}>
                  {(exp) => (
                    <span
                      data-testid="currency-expected"
                      data-expected={exp()}
                      class="xc-currency-count-expected"
                    >
                      {t('expected', { amount: formatAmount(exp()) })}
                    </span>
                  )}
                </Show>
                <Show when={!props.locked.includes(code)}>
                  <button
                    type="button"
                    data-testid="currency-remove"
                    class="xc-currency-count-remove"
                    onClick={() => removeCurrency(code)}
                  >
                    {t('remove')}
                  </button>
                </Show>
              </div>
              <div class="xc-shift-denoms">
                <For each={props.denominations.filter((d) => d.currency === code)}>
                  {(d) => (
                    <div data-denomination-row data-face-value={d.faceValue} class="xc-shift-denom-row">
                      <span class="xc-shift-denom-label">
                        <span class="xc-shift-denom-currency">{d.currency}</span>{' '}
                        <span class="xc-shift-denom-val">{formatFaceValue(d.faceValue)}</span>
                      </span>
                      <input
                        data-denomination-id={d.id}
                        class="xc-shift-denom-input"
                        type="text"
                        inputMode="numeric"
                        value={props.quantities[d.id] ?? ''}
                        onInput={(e) => props.setQuantity(d.id, e.currentTarget.value)}
                        onChange={(e) => props.setQuantity(d.id, e.currentTarget.value)}
                      />
                      <Show when={props.lineErrors[d.id]}>
                        {(msg) => <span data-field-error={`lines.${d.id}.qty`}>{msg()}</span>}
                      </Show>
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
