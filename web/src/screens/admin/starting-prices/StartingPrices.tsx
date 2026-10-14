// SP-3 (#1617): Admin Starting Prices screen (design `docs/design/blueprint/rates.md` §12.6, §12.7, §12.10 SP-3).
import { createEffect, createSignal, For, Show, untrack, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  OrgStartingPriceSaveInput,
  OrgStartingPriceSaveOutput,
  UnpricedBranch,
} from '../../../shared/api/generated/org_starting_price_save';
import type { OrgStartingPriceItem } from '../../../shared/api/generated/org_starting_prices';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './starting-prices.css';

const t = screenT('admin.starting-prices', table);

/** The refusals this screen words itself; any other shows through wordedError. */
const OWN_TEXT: Record<string, Parameters<typeof t>[0]> = {
  idempotencyConflict: 'error.idempotencyConflict',
  validationFailed: 'error.validationFailed',
};

function startingPricesError(err: unknown): string {
  if (err instanceof ApiError && err.status < 500 && err.code in OWN_TEXT) {
    return t(OWN_TEXT[err.code]!);
  }
  return wordedError(err);
}

export function isValidPositiveDecimal(val: string): boolean {
  if (!/^\d+(\.\d+)?$/.test(val)) return false;
  const [intPart, fracPart = ''] = val.split('.');
  const allZeros = /^0+$/.test(intPart) && (fracPart === '' || /^0+$/.test(fracPart));
  return !allZeros;
}

export function isBuyGreaterThanSell(buy: string, sell: string): boolean {
  const [buyIntStr, buyFracStr = ''] = buy.split('.');
  const [sellIntStr, sellFracStr = ''] = sell.split('.');

  const buyInt = buyIntStr.replace(/^0+/, '') || '0';
  const sellInt = sellIntStr.replace(/^0+/, '') || '0';

  if (buyInt.length > sellInt.length) return true;
  if (buyInt.length < sellInt.length) return false;
  if (buyInt > sellInt) return true;
  if (buyInt < sellInt) return false;

  const maxFracLen = Math.max(buyFracStr.length, sellFracStr.length);
  const buyFracPadded = buyFracStr.padEnd(maxFracLen, '0');
  const sellFracPadded = sellFracStr.padEnd(maxFracLen, '0');

  return buyFracPadded > sellFracPadded;
}

function formatWhen(iso: string): string {
  try {
    return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
  } catch {
    return iso;
  }
}

export function StartingPrices(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [items, setItems] = createSignal<OrgStartingPriceItem[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);

  const [addedCurrencies, setAddedCurrencies] = createSignal<string[]>([]);
  const [typedBuy, setTypedBuy] = createSignal<Record<string, string>>({});
  const [typedSell, setTypedSell] = createSignal<Record<string, string>>({});
  const [reason, setReason] = createSignal('');

  const [expandedHistory, setExpandedHistory] = createSignal<Record<string, boolean>>({});
  const [addCode, setAddCode] = createSignal('');
  const [addCodeError, setAddCodeError] = createSignal<string | null>(null);

  const [busy, setBusy] = createSignal(false);
  const [formError, setFormError] = createSignal<string | null>(null);
  const [reasonError, setReasonError] = createSignal<string | null>(null);
  const [rowErrors, setRowErrors] = createSignal<Record<string, string>>({});
  const [buyErrors, setBuyErrors] = createSignal<Record<string, string>>({});
  const [sellErrors, setSellErrors] = createSignal<Record<string, string>>({});
  const [unpriced, setUnpriced] = createSignal<UnpricedBranch[]>([]);

  // History and currencies
  const currencyGroups = () => {
    const map = new Map<string, OrgStartingPriceItem[]>();
    for (const item of items()) {
      const list = map.get(item.currency) ?? [];
      list.push(item);
      map.set(item.currency, list);
    }
    for (const list of map.values()) {
      list.sort((a, b) => b.version - a.version);
    }
    return map;
  };

  const allCurrencies = () => {
    const set = new Set<string>();
    for (const item of items()) {
      set.add(item.currency);
    }
    for (const code of addedCurrencies()) {
      set.add(code);
    }
    return [...set].sort();
  };

  async function loadView(resetTyped = false) {
    setLoading(true);
    setLoadError(null);
    try {
      const res = await api.view<OrgStartingPriceItem[]>('org_starting_prices');
      const data = Array.isArray(res.data) ? res.data : [];
      setItems(data);

      const map = new Map<string, OrgStartingPriceItem[]>();
      for (const item of data) {
        const list = map.get(item.currency) ?? [];
        list.push(item);
        map.set(item.currency, list);
      }

      const currentBuys: Record<string, string> = { ...typedBuy() };
      const currentSells: Record<string, string> = { ...typedSell() };

      for (const [cur, list] of map.entries()) {
        list.sort((a, b) => b.version - a.version);
        const latest = list[0];
        if (resetTyped || currentBuys[cur] === undefined) {
          currentBuys[cur] = latest.buy ?? '';
        }
        if (resetTyped || currentSells[cur] === undefined) {
          currentSells[cur] = latest.sell ?? '';
        }
      }
      setTypedBuy(currentBuys);
      setTypedSell(currentSells);
    } catch (err: unknown) {
      setLoadError(startingPricesError(err));
    } finally {
      setLoading(false);
    }
  }

  let opened = false;
  createEffect(() => {
    if (!props.shown()) return;
    untrack(() => {
      if (opened) return;
      opened = true;
      void loadView(true);
    });
  });

  function handleAddCurrency(e?: Event) {
    e?.preventDefault();
    setAddCodeError(null);
    const code = addCode().trim().toUpperCase();
    if (!code) return;

    if (allCurrencies().includes(code)) {
      setAddCode('');
      return;
    }

    setAddedCurrencies((prev) => [...prev, code]);
    setTypedBuy((prev) => ({ ...prev, [code]: '' }));
    setTypedSell((prev) => ({ ...prev, [code]: '' }));
    setAddCode('');
  }

  // Idempotency-Key retention
  let pendingKey: { bodyString: string; key: string } | null = null;

  async function handleSave(e: Event) {
    e.preventDefault();
    if (busy()) return;

    setFormError(null);
    setRowErrors({});
    setBuyErrors({});
    setSellErrors({});

    const trimmedReason = reason().trim();
    let hasErrors = false;

    if (!trimmedReason) {
      setReasonError(t('validation.reasonRequired'));
      hasErrors = true;
    } else {
      setReasonError(null);
    }

    const newRowErrors: Record<string, string> = {};
    const newBuyErrors: Record<string, string> = {};
    const newSellErrors: Record<string, string> = {};
    const changedPrices: { currency: string; buy: string | null; sell: string | null }[] = [];

    const groups = currencyGroups();

    for (const cur of allCurrencies()) {
      const latestItem = groups.get(cur)?.[0];
      const initBuy = latestItem?.buy ?? null;
      const initSell = latestItem?.sell ?? null;

      const rawBuy = typedBuy()[cur] ?? '';
      const rawSell = typedSell()[cur] ?? '';
      const buyVal = rawBuy.trim() === '' ? null : rawBuy.trim();
      const sellVal = rawSell.trim() === '' ? null : rawSell.trim();

      const isChanged = !latestItem || buyVal !== initBuy || sellVal !== initSell;
      if (!isChanged) continue;

      let rowHasError = false;

      // Both empty check
      if (buyVal === null && sellVal === null) {
        newRowErrors[cur] = t('validation.emptyRow');
        rowHasError = true;
      }

      // Buy positive decimal check
      if (buyVal !== null && !isValidPositiveDecimal(buyVal)) {
        newBuyErrors[cur] = t('validation.positiveDecimal');
        rowHasError = true;
      }

      // Sell positive decimal check
      if (sellVal !== null && !isValidPositiveDecimal(sellVal)) {
        newSellErrors[cur] = t('validation.positiveDecimal');
        rowHasError = true;
      }

      // Buy > sell check
      if (
        buyVal !== null &&
        sellVal !== null &&
        isValidPositiveDecimal(buyVal) &&
        isValidPositiveDecimal(sellVal) &&
        isBuyGreaterThanSell(buyVal, sellVal)
      ) {
        newRowErrors[cur] = t('validation.buyGreaterThanSell');
        rowHasError = true;
      }

      if (rowHasError) {
        hasErrors = true;
      } else {
        changedPrices.push({
          currency: cur,
          buy: buyVal,
          sell: sellVal,
        });
      }
    }

    setRowErrors(newRowErrors);
    setBuyErrors(newBuyErrors);
    setSellErrors(newSellErrors);

    if (hasErrors) return;

    if (changedPrices.length === 0) {
      setFormError(t('nothingToSave'));
      return;
    }

    const body: OrgStartingPriceSaveInput = {
      prices: changedPrices,
      reason: trimmedReason,
    };
    const bodyString = JSON.stringify(body);
    if (!pendingKey || pendingKey.bodyString !== bodyString) {
      pendingKey = {
        bodyString,
        key: crypto.randomUUID(),
      };
    }
    const key = pendingKey.key;
    setBusy(true);

    try {
      const res = await api.command<OrgStartingPriceSaveInput, OrgStartingPriceSaveOutput>(
        'POST',
        '/api/v3/commands/org_starting_price_save',
        body,
        key,
      );
      pendingKey = null;
      toast(t('saved'));
      setAddedCurrencies([]);
      setUnpriced(res?.unpriced ?? []);
      await loadView(true);
    } catch (err: unknown) {
      if (err instanceof ApiError && err.status < 500) {
        pendingKey = null;
        if (err.field === 'currency') {
          const rErrors: Record<string, string> = {};
          for (const p of changedPrices) {
            rErrors[p.currency] = t('error.currency');
          }
          setRowErrors(rErrors);
        } else if (err.field === 'reason') {
          setReasonError(t('error.reason'));
        } else {
          setFormError(startingPricesError(err));
        }
      } else {
        setFormError(startingPricesError(err));
      }
    } finally {
      setBusy(false);
    }
  }

  function formatUnpriced(branch: string, unpricedReason: string): string {
    if (unpricedReason === 'noPolicy') {
      return t('unpricedNoPolicy', { branch });
    }
    return t('unpricedGeneric', { branch, reason: unpricedReason });
  }

  return (
    <div class="xc-sp-screen" data-section="starting-prices">
      <Show when={loading()}>
        <div class="xc-sp-loading">{t('loading')}</div>
      </Show>

      <Show when={loadError()}>
        <div class="xc-sp-error" role="alert">
          <span>{loadError()}</span>
          <button type="button" class="xc-sp-retry-btn" onClick={() => void loadView(false)}>
            {t('retry')}
          </button>
        </div>
      </Show>

      <Show when={unpriced().length > 0}>
        <div class="xc-sp-unpriced-list">
          <div class="xc-sp-unpriced-title">{t('unpricedTitle')}</div>
          <ul>
            <For each={unpriced()}>
              {(item) => (
                <li class="xc-sp-unpriced-item">
                  {formatUnpriced(item.branch, item.reason)}
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>

      <Show when={formError()}>
        <div class="xc-sp-form-error" role="alert">
          {formError()}
        </div>
      </Show>

      <form class="xc-sp-form" onSubmit={handleSave}>
        <Show when={allCurrencies().length === 0}>
          <div class="xc-sp-empty">{t('empty')}</div>
        </Show>

        <Show when={allCurrencies().length > 0}>
          <div class="xc-sp-table-scroll">
            <table class="xc-sp-table">
              <thead>
                <tr>
                  <th>{t('headerCurrency')}</th>
                  <th>{t('headerCurrentBuy')} / {t('headerCurrentSell')}</th>
                  <th>{t('headerNewBuy')}</th>
                  <th>{t('headerNewSell')}</th>
                </tr>
              </thead>
              <tbody>
                <For each={allCurrencies()}>
                  {(cur) => {
                    const historyList = () => currencyGroups().get(cur) ?? [];
                    const latestItem = () => historyList()[0];
                    const olderVersions = () => historyList().slice(1);
                    const isExpanded = () => !!expandedHistory()[cur];

                    return (
                      <tr class="xc-sp-currency-row" data-currency={cur}>
                        <td>
                          <div class="xc-sp-currency-code">{cur}</div>
                          <Show when={latestItem()}>
                            {(item) => (
                              <div class="xc-sp-history-meta">
                                v{item().version} · {t('headerBy')} {item().enteredBy} · {formatWhen(item().enteredAt)} ({item().reason})
                              </div>
                            )}
                          </Show>
                          <Show when={olderVersions().length > 0}>
                            <button
                              type="button"
                              class="xc-sp-history-toggle"
                              onClick={() => setExpandedHistory((prev) => ({ ...prev, [cur]: !prev[cur] }))}
                            >
                              {isExpanded() ? t('historyHide') : t('historyShow')}
                            </button>
                            <Show when={isExpanded()}>
                              <div class="xc-sp-history-panel">
                                <For each={olderVersions()}>
                                  {(old) => (
                                    <div class="xc-sp-history-item">
                                      v{old.version}: {old.buy ?? t('dash')} / {old.sell ?? t('dash')} · {old.enteredBy} · {formatWhen(old.enteredAt)} ({old.reason})
                                    </div>
                                  )}
                                </For>
                              </div>
                            </Show>
                          </Show>
                          <Show when={rowErrors()[cur]}>
                            <div class="xc-sp-row-error xc-sp-currency-error">{rowErrors()[cur]}</div>
                          </Show>
                        </td>
                        <td>
                          <div class="xc-sp-current-prices">
                            <span class="xc-sp-current-price-item">
                              {t('headerCurrentBuy')}: {latestItem()?.buy ?? t('dash')}
                            </span>
                            <span class="xc-sp-current-price-item">
                              {t('headerCurrentSell')}: {latestItem()?.sell ?? t('dash')}
                            </span>
                          </div>
                        </td>
                        <td class="xc-sp-input-cell">
                          <input
                            type="text"
                            inputMode="decimal"
                            class="xc-sp-input"
                            data-field="buy"
                            value={typedBuy()[cur] ?? ''}
                            onInput={(e) => {
                              const val = e.currentTarget.value;
                              setTypedBuy((prev) => ({ ...prev, [cur]: val }));
                            }}
                          />
                          <Show when={buyErrors()[cur]}>
                            <div class="xc-sp-buy-error" data-error-for="buy">{buyErrors()[cur]}</div>
                          </Show>
                        </td>
                        <td class="xc-sp-input-cell">
                          <input
                            type="text"
                            inputMode="decimal"
                            class="xc-sp-input"
                            data-field="sell"
                            value={typedSell()[cur] ?? ''}
                            onInput={(e) => {
                              const val = e.currentTarget.value;
                              setTypedSell((prev) => ({ ...prev, [cur]: val }));
                            }}
                          />
                          <Show when={sellErrors()[cur]}>
                            <div class="xc-sp-sell-error" data-error-for="sell">{sellErrors()[cur]}</div>
                          </Show>
                        </td>
                      </tr>
                    );
                  }}
                </For>
              </tbody>
            </table>
          </div>
        </Show>

        <div class="xc-sp-add-currency">
          <input
            type="text"
            class="xc-sp-add-code-input"
            maxLength={3}
            placeholder={t('addPlaceholder')}
            value={addCode()}
            onInput={(e) => setAddCode(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault();
                handleAddCurrency();
              }
            }}
          />
          <button type="button" class="xc-sp-add-code-btn" onClick={() => handleAddCurrency()}>
            {t('addCurrency')}
          </button>
          <Show when={addCodeError()}>
            <div class="xc-sp-add-error">{addCodeError()}</div>
          </Show>
        </div>

        <div class="xc-sp-reason-field">
          <label for="xc-sp-reason" class="xc-sp-reason-label">
            {t('reasonLabel')}
          </label>
          <input
            id="xc-sp-reason"
            name="reason"
            type="text"
            class="xc-sp-reason-input"
            placeholder={t('reasonPlaceholder')}
            value={reason()}
            onInput={(e) => setReason(e.currentTarget.value)}
          />
          <Show when={reasonError()}>
            <div class="xc-sp-reason-error">{reasonError()}</div>
          </Show>
        </div>

        <div class="xc-sp-actions">
          <button
            type="submit"
            class="xc-sp-save-btn"
            data-action="save"
            disabled={busy()}
          >
            {busy() ? t('saving') : t('save')}
          </button>
        </div>
      </form>
    </div>
  );
}
