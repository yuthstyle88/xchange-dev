import {
  createEffect,
  createMemo,
  createSignal,
  For,
  onMount,
  Show,
  type JSX,
} from 'solid-js';
import { ApiError, type ApiClient } from '../../shared/api/client';
import type {
  KbankReference,
  KioskCurrencies,
  KioskCurrency,
} from '../../shared/api/generated/kiosk_currencies';
import { errorText, locale, screenT } from '../../shared/i18n';
import { type FieldBinding, useForm } from '../../shared/ui/form';
import { TextField } from '../../shared/ui/TextField';
import { matches } from './filter';
import * as screenI18n from './i18n';
import { configure } from './kiosk-api';
import './kiosk-currencies.css';

type CardStatusState =
  | { kind: 'savedOn' }
  | { kind: 'savedOff' }
  | { kind: 'message'; text: string };

interface CardProps {
  currency: KioskCurrency;
  status: () => string;
  saving: () => boolean;
  kbankRef: () => KbankReference | null;
  t: ReturnType<typeof screenT<typeof screenI18n>>;
  onSave: (
    code: string,
    buyVal: string,
    sellVal: string,
    isEnabled: boolean,
    previousUpdated: string,
    partnerName: string,
    partnerCost: string | null,
  ) => Promise<void>;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function CurrencyCard(props: CardProps): JSX.Element {
  const form = useForm();
  const [enabled, setEnabled] = createSignal(props.currency.enabled);

  createEffect(() => {
    setEnabled(props.currency.enabled);
  });

  let buyInput: HTMLInputElement | undefined;
  let sellInput: HTMLInputElement | undefined;

  const buyField = form.field('buy');
  const origBuyRef = buyField.ref;
  const buyBinding: FieldBinding = {
    ...buyField,
    ref: (el: HTMLInputElement) => {
      origBuyRef(el);
      buyInput = el;
      if (el) {
        el.value = props.currency.buy ?? '';
      }
    },
  };

  const sellField = form.field('sell');
  const origSellRef = sellField.ref;
  const sellBinding: FieldBinding = {
    ...sellField,
    ref: (el: HTMLInputElement) => {
      origSellRef(el);
      sellInput = el;
      if (el) {
        el.value = props.currency.sell ?? '';
      }
    },
  };

  createEffect(() => {
    if (buyInput) {
      buyInput.value = props.currency.buy ?? '';
    }
    if (sellInput) {
      sellInput.value = props.currency.sell ?? '';
    }
  });

  const handleSubmit = form.submit(async () => {
    const buyVal = buyInput?.value ?? '';
    const sellVal = sellInput?.value ?? '';
    const isEnabled = enabled();

    await props.onSave(
      props.currency.code,
      buyVal,
      sellVal,
      isEnabled,
      props.currency.updated,
      props.currency.partnerName || '',
      props.currency.partnerCost ?? null,
    );
  });

  return (
    <form class="quote" onSubmit={handleSubmit}>
      <h3>
        {props.currency.code} · {props.currency.name} / {props.currency.nameEn}
      </h3>
      <p>{props.currency.countries.map((c) => c.th).join(', ')}</p>
      <div>
        <label for={`enabled-${props.currency.code}`}>
          <input
            id={`enabled-${props.currency.code}`}
            type="checkbox"
            checked={enabled()}
            onChange={(e) => setEnabled(e.currentTarget.checked)}
          />
          {props.t('enabled')}
        </label>
      </div>

      <TextField
        field={buyBinding}
        label={props.t('buy')}
        type="number"
        required
        min="0.0001"
        max="1000000"
        step="0.0001"
        inputMode="decimal"
      />

      <TextField
        field={sellBinding}
        label={props.t('sell')}
        type="number"
        required
        min="0.0001"
        max="1000000"
        step="0.0001"
        inputMode="decimal"
      />

      <div class="kbank-box">
        <Show
          when={props.kbankRef()}
          fallback={<div>{props.t('kbLoading')}</div>}
        >
          {(ref) => (
            <>
              <h4>{props.t('kbCaption')}</h4>
              <table>
                <thead>
                  <tr>
                    <th>{props.t('colLabel')}</th>
                    <th>{props.t('kbColBuy')}</th>
                    <th>{props.t('kbColSell')}</th>
                    <th>{props.t('kbColTime')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={props.currency.kbank}>
                    {(r) => (
                      <tr>
                        <td>{r.label}</td>
                        <td>{r.bankBuy ?? '—'}</td>
                        <td>{r.bankSell ?? '—'}</td>
                        <td>
                          {props.t('kbStamp', {
                            time: formatTime(ref().publishedAt),
                            round: ref().round,
                          })}
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
              <p>{props.t('kbNote')}</p>
            </>
          )}
        </Show>
      </div>

      <button type="submit" class="primary" disabled={props.saving()}>
        {props.t('save')}
      </button>

      <span role="status">{props.status()}</span>
    </form>
  );
}

export function KioskCurrenciesScreen(props: {
  api: Pick<ApiClient, 'view' | 'post'>;
  branch: string;
}): JSX.Element {
  const t = screenT('kiosk-currencies', screenI18n);

  const [currencies, setCurrencies] = createSignal<KioskCurrency[]>([]);
  const [kbankRef, setKbankRef] = createSignal<KbankReference | null>(null);
  const [searchQuery, setSearchQuery] = createSignal('');
  const [loadError, setLoadError] = createSignal<
    { code: string; message: string } | undefined
  >(undefined);
  const [cardStatuses, setCardStatuses] = createSignal<
    Record<string, CardStatusState | undefined>
  >({});
  const [savingCodes, setSavingCodes] = createSignal<Record<string, boolean>>({});

  onMount(async () => {
    try {
      const res = await props.api.view<KioskCurrencies>('kiosk_currencies', {
        branch: props.branch,
      });
      if (res.data) {
        setCurrencies(res.data.currencies);
        setKbankRef(res.data.kbank);
      }
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setLoadError({ code: err.code, message: err.message });
      } else if (err instanceof Error) {
        setLoadError({ code: 'error', message: err.message });
      } else {
        setLoadError({ code: 'error', message: String(err) });
      }
      setCurrencies([]);
      setKbankRef(null);
    }
  });

  const filteredCurrencies = createMemo(() => {
    const q = searchQuery();
    return currencies().filter((c) => matches(c, q));
  });

  const pageMessage = createMemo(() => {
    const err = loadError();
    if (err) {
      return errorText(err.code, err.message);
    }
    return t('found', { n: filteredCurrencies().length });
  });

  const handleSave = async (
    code: string,
    buyVal: string,
    sellVal: string,
    isEnabled: boolean,
    previousUpdated: string,
    partnerName: string,
    partnerCost: string | null,
  ) => {
    setSavingCodes((prev) => ({ ...prev, [code]: true }));
    setCardStatuses((prev) => ({ ...prev, [code]: undefined }));

    try {
      await configure(props.api, {
        branchId: props.branch || 'MAIN',
        code,
        buy: buyVal,
        sell: sellVal,
        enabled: isEnabled,
        previousUpdated,
        partnerName,
        partnerCost,
      });

      const res = await props.api.view<KioskCurrencies>('kiosk_currencies', {
        branch: props.branch,
      });

      if (res.data) {
        setCurrencies(res.data.currencies);
        setKbankRef(res.data.kbank);
        const thisCurrency = res.data.currencies.find((c) => c.code === code);
        const active = thisCurrency ? thisCurrency.enabled : isEnabled;
        setCardStatuses((prev) => ({
          ...prev,
          [code]: active ? { kind: 'savedOn' } : { kind: 'savedOff' },
        }));
      }
    } catch (err: unknown) {
      const msg =
        err instanceof ApiError || err instanceof Error
          ? err.message
          : String(err);
      setCardStatuses((prev) => ({
        ...prev,
        [code]: { kind: 'message', text: msg },
      }));
    } finally {
      setSavingCodes((prev) => ({ ...prev, [code]: false }));
    }
  };

  const getCardStatusText = (code: string) => {
    const st = cardStatuses()[code];
    if (!st) return '';
    if (st.kind === 'savedOn') return t('savedOn');
    if (st.kind === 'savedOff') return t('savedOff');
    return st.text;
  };

  return (
    <div>
      <a href="/">{t('back')}</a>
      <h1>{t('heading')}</h1>
      <p>{t('intro')}</p>

      <section aria-label={t('bankReference')} id="bank-reference">
        <Show
          when={kbankRef()}
          fallback={<p>{t('kbankFailed')}</p>}
        >
          {(ref) => (
            <>
              <h3>{t('compareHeading')}</h3>
              <p>
                {t('comparePublished', {
                  time: formatTime(ref().publishedAt),
                  round: ref().round,
                })}
              </p>
              <p class="warning">{t('compareWarning')}</p>
              <p>
                <a href={ref().source} target="_blank" rel="noopener noreferrer">
                  {t('compareLink')}
                </a>
              </p>
              <p>{t('compareNote')}</p>
              <table>
                <thead>
                  <tr>
                    <th>{t('colLabel')}</th>
                    <th>{t('cmpColBankBuy')}</th>
                    <th>{t('cmpColShopBuy')}</th>
                    <th>{t('cmpColBuyDiff')}</th>
                    <th>{t('cmpColBankSell')}</th>
                    <th>{t('cmpColShopSell')}</th>
                    <th>{t('cmpColSellDiff')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={filteredCurrencies()}>
                    {(c) => (
                      <For each={c.kbank}>
                        {(r) => (
                          <tr>
                            <td>{r.label}</td>
                            <td>{r.bankBuy ?? '—'}</td>
                            <td>{r.shopBuy ?? '—'}</td>
                            <td>{r.buyDifference ?? '—'}</td>
                            <td>{r.bankSell ?? '—'}</td>
                            <td>{r.shopSell ?? '—'}</td>
                            <td>{r.sellDifference ?? '—'}</td>
                          </tr>
                        )}
                      </For>
                    )}
                  </For>
                </tbody>
              </table>
            </>
          )}
        </Show>
      </section>

      <div>
        <label for="kiosk-search">{t('searchLabel')}</label>
        <input
          id="kiosk-search"
          type="search"
          placeholder={t('searchPlaceholder')}
          value={searchQuery()}
          onInput={(e) => setSearchQuery(e.currentTarget.value)}
        />
      </div>

      <div role="status">{pageMessage()}</div>

      <div class="cards">
        <For each={filteredCurrencies()}>
          {(c) => (
            <CurrencyCard
              currency={c}
              status={() => getCardStatusText(c.code)}
              saving={() => Boolean(savingCodes()[c.code])}
              kbankRef={kbankRef}
              t={t}
              onSave={handleSave}
            />
          )}
        </For>
      </div>
    </div>
  );
}
