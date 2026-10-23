import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Show,
  type JSX,
} from 'solid-js';
import type { ApiClient } from '../../shared/api/client';
import type { RateFreshnessDto } from '../../shared/api/generated/common';
import type {
  KbankReference,
  KioskCurrencies,
  KioskCurrency,
} from '../../shared/api/generated/kiosk_currencies';
import { lang, locale, screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { formatRate } from '../../shared/ui/rate';
import { RateFreshness } from '../../shared/ui/RateFreshness';
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
  onToggle: (code: string, isEnabled: boolean) => Promise<void>;
}

function formatTime(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function CurrencyCard(props: CardProps): JSX.Element {
  let checkboxRef: HTMLInputElement | undefined;

  createEffect(() => {
    if (checkboxRef) {
      checkboxRef.checked = props.currency.enabled;
    }
  });

  const handleToggle = async (e: Event) => {
    const target = e.currentTarget as HTMLInputElement;
    const isEnabled = target.checked;
    await props.onToggle(props.currency.code, isEnabled);
    if (checkboxRef) {
      checkboxRef.checked = props.currency.enabled;
    }
  };

  return (
    <form class="quote" onSubmit={(e) => e.preventDefault()}>
      <h3>
        {props.currency.code} · {props.currency.name} / {props.currency.nameEn}
      </h3>
      <p>{props.currency.countries.map((c) => c.th).join(', ')}</p>
      <div>
        <label for={`enabled-${props.currency.code}`}>
          <input
            id={`enabled-${props.currency.code}`}
            type="checkbox"
            ref={checkboxRef}
            checked={props.currency.enabled}
            disabled={props.saving()}
            onChange={handleToggle}
          />
          {props.t('enabled')}
        </label>
      </div>

      <div class="rate-side">
        <span>{props.t('buy')}</span>
        <span>
          {props.currency.buy
            ? formatRate(props.currency.buy, props.currency.unitSize, props.currency.code)
            : '—'}
        </span>
      </div>

      <div class="rate-side">
        <span>{props.t('sell')}</span>
        <span>
          {props.currency.sell
            ? formatRate(props.currency.sell, props.currency.unitSize, props.currency.code)
            : '—'}
        </span>
      </div>

      <div class="kbank-box">
        <Show
          when={props.kbankRef()}
          fallback={<div>{props.t('kbLoading')}</div>}
        >
          {(ref) => (
            <>
              <div class="kbank-table">
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
              </div>
              <p>{props.t('kbNote')}</p>
            </>
          )}
        </Show>
      </div>

      <span role="status">{props.status()}</span>
    </form>
  );
}

export function KioskCurrenciesScreen(props: {
  api: Pick<ApiClient, 'view' | 'post'>;
  branch?: string;
}): JSX.Element {
  const t = screenT('kiosk-currencies', screenI18n);

  const [currencies, setCurrencies] = createSignal<KioskCurrency[]>([]);
  const [kbankRef, setKbankRef] = createSignal<KbankReference | null>(null);
  const [freshness, setFreshness] = createSignal<RateFreshnessDto | null>(null);
  const [searchQuery, setSearchQuery] = createSignal('');
  const [loadError, setLoadError] = createSignal<string | undefined>(undefined);
  const [loaded, setLoaded] = createSignal(false);
  const [cardStatuses, setCardStatuses] = createSignal<
    Record<string, CardStatusState | undefined>
  >({});
  const [savingCodes, setSavingCodes] = createSignal<Record<string, boolean>>({});

  let readingBranch = '';
  // Bumped by every list read (first read and toggle re-read); a list read
  // (first read or toggle re-read) applies only if it is still the latest.
  // When a newer toggle re-read supersedes the first read, that re-read
  // ends the loading state.
  let listReadSeq = 0;
  const fetchCurrencies = async (branchId: string) => {
    readingBranch = branchId;
    listReadSeq += 1;
    const seq = listReadSeq;
    setLoaded(false);
    setCurrencies([]);
    setKbankRef(null);
    setFreshness(null);
    setLoadError(undefined);
    setCardStatuses({});
    setSavingCodes({});
    try {
      const res = await props.api.view<KioskCurrencies>('kiosk_currencies', {
        branch: branchId,
      });
      if (readingBranch !== branchId) return;
      if (seq !== listReadSeq) return;
      if (res.data) {
        setCurrencies(res.data.currencies);
        setKbankRef(res.data.kbank);
        setFreshness(res.data.freshness);
      }
      setLoaded(true);
    } catch (err: unknown) {
      if (readingBranch !== branchId) return;
      if (seq !== listReadSeq) return;
      setLoadError(wordedError(err));
      setCurrencies([]);
      setKbankRef(null);
      setFreshness(null);
    }
  };

  createEffect(() => {
    const b = props.branch;
    setResultSecondsInput('');
    setSettingsError(undefined);
    setResultSecondsDirty(false);
    setKnown(false);
    if (!b) {
      setLoaded(false);
      setCurrencies([]);
      setKbankRef(null);
      setFreshness(null);
      setLoadError(undefined);
      setCardStatuses({});
      setSavingCodes({});
      return;
    }
    void fetchCurrencies(b);
  });

  const filteredCurrencies = createMemo(() => {
    const q = searchQuery();
    return currencies().filter((c) => matches(c, q));
  });

  const pageMessage = createMemo(() => {
    const err = loadError();
    if (err) {
      return err;
    }
    if (!loaded()) {
      return '';
    }
    return t('found', { n: filteredCurrencies().length });
  });

  const handleToggle = async (code: string, isEnabled: boolean) => {
    const branchId = props.branch;
    if (!branchId) return;
    setSavingCodes((prev) => ({ ...prev, [code]: true }));
    setCardStatuses((prev) => ({ ...prev, [code]: undefined }));

    try {
      await configure(props.api, {
        branchId,
        code,
        enabled: isEnabled,
      });
    } catch (err: unknown) {
      if (props.branch === branchId) {
        const msg = wordedError(err);
        setCardStatuses((prev) => ({
          ...prev,
          [code]: { kind: 'message', text: msg },
        }));
        setSavingCodes((prev) => ({ ...prev, [code]: false }));
      }
      return;
    }

    if (props.branch !== branchId) {
      return;
    }

    listReadSeq += 1;
    const seq = listReadSeq;
    try {
      const res = await props.api.view<KioskCurrencies>('kiosk_currencies', {
        branch: branchId,
      });
      if (props.branch !== branchId) return;
      const latest = seq === listReadSeq;

      if (latest) setLoadError(undefined);
      if (res.data) {
        if (latest) {
          setCurrencies(res.data.currencies);
          setKbankRef(res.data.kbank);
          setFreshness(res.data.freshness);
        }
        const thisCurrency = res.data.currencies.find((c) => c.code === code);
        const active = thisCurrency ? thisCurrency.enabled : isEnabled;
        setCardStatuses((prev) => ({
          ...prev,
          [code]: active ? { kind: 'savedOn' } : { kind: 'savedOff' },
        }));
      }
      // This re-read may have superseded the first read: it ends loading.
      if (latest) setLoaded(true);
    } catch (err: unknown) {
      if (props.branch !== branchId) return;
      if (seq === listReadSeq) {
        setCurrencies((prev) =>
          prev.map((c) => (c.code === code ? { ...c, enabled: isEnabled } : c)),
        );
        setLoadError(wordedError(err));
      }
      setCardStatuses((prev) => ({
        ...prev,
        [code]: isEnabled ? { kind: 'savedOn' } : { kind: 'savedOff' },
      }));
    } finally {
      if (props.branch === branchId) {
        setSavingCodes((prev) => ({ ...prev, [code]: false }));
      }
    }
  };

  const getCardStatusText = (code: string) => {
    const st = cardStatuses()[code];
    if (!st) return '';
    if (st.kind === 'savedOn') return t('savedOn');
    if (st.kind === 'savedOff') return t('savedOff');
    return st.text;
  };

  const [resultSecondsInput, setResultSecondsInput] = createSignal<string>('');
  const [settingsError, setSettingsError] = createSignal<string | undefined>();
  const [savingSettings, setSavingSettings] = createSignal<boolean>(false);
  const [resultSecondsDirty, setResultSecondsDirty] = createSignal<boolean>(false);
  const [known, setKnown] = createSignal<boolean>(false);

  const handleReadSettings = async () => {
    const branchId = props.branch;
    if (!branchId) return;
    try {
      const res = await props.api.post<
        { action: string; payload: { branchId: string } },
        { resultSeconds: number; version: number }
      >('/api/kiosk', {
        action: 'settings',
        payload: { branchId },
      });
      if (props.branch === branchId) {
        // A read never overwrites what the admin has typed since.
        if (!resultSecondsDirty()) setResultSecondsInput(String(res.resultSeconds));
        setKnown(true);
        setSettingsError(undefined);
      }
    } catch (err: unknown) {
      if (props.branch === branchId) setSettingsError(wordedError(err));
    }
  };

  const handleSaveSettings = async (e?: Event) => {
    e?.preventDefault();
    const branchId = props.branch;
    if (!branchId || !known()) return;
    const raw = resultSecondsInput().trim();
    if (!/^[0-9]+$/.test(raw)) {
      setSettingsError(t('resultSecondsRange'));
      return;
    }
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed) || parsed < 3 || parsed > 120) {
      setSettingsError(t('resultSecondsRange'));
      return;
    }
    setSavingSettings(true);
    setSettingsError(undefined);
    try {
      const res = await props.api.post<
        { action: string; payload: { branchId: string; resultSeconds: number } },
        { resultSeconds: number; version: number }
      >('/api/kiosk', {
        action: 'configureSettings',
        payload: {
          branchId,
          resultSeconds: parsed,
        },
      });
      if (props.branch === branchId) {
        setResultSecondsInput(String(res.resultSeconds));
        setResultSecondsDirty(false);
      }
    } catch (err: unknown) {
      if (props.branch === branchId) {
        const errObj = err as any;
        const isValidation =
          errObj?.status === 400 ||
          errObj?.code === 'validationFailed' ||
          errObj?.message === 'validationFailed';
        let msg = '';
        if (isValidation) {
          if (
            typeof errObj?.message === 'string' &&
            errObj.message.trim() &&
            errObj.message !== 'validationFailed' &&
            errObj.message !== 'refused'
          ) {
            msg = errObj.message;
          } else {
            msg = t('resultSecondsRange');
          }
        } else {
          msg = wordedError(err);
        }
        setSettingsError(msg);
      }
    } finally {
      if (props.branch === branchId) {
        setSavingSettings(false);
      }
    }
  };

  return (
    <div>
      <a href="/">{t('back')}</a>
      <h1>{t('heading')}</h1>
      <p>{t('intro')}</p>

      <RateFreshness
        stale={freshness()?.stale ?? false}
        sourceDate={freshness()?.sourceDate}
        publishedAt={freshness()?.publishedAt}
      />

      <section aria-label={t('bankReference')} id="bank-reference">
        <Show
          when={kbankRef()}
          fallback={loaded() || loadError() ? t('kbankFailed') : ''}
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
              <div class="bank-compare">
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
              </div>
            </>
          )}
        </Show>
      </section>

      <section aria-label={lang() === 'en' ? 'Queue machine settings' : 'ตั้งค่าตู้คิว'}>
        <div>
          <label for="result_seconds">
            {lang() === 'en' ? 'Result display time (seconds)' : 'เวลาแสดงผลบัตรคิว (วินาที)'}
          </label>
          <div style="display: flex; gap: 8px; align-items: center;">
            <input
              id="result_seconds"
              name="result_seconds"
              type="text"
              inputmode="numeric"
              pattern="[0-9]*"
              value={resultSecondsInput()}
              disabled={savingSettings()}
              aria-invalid={settingsError() ? 'true' : 'false'}
              onFocus={handleReadSettings}
              onInput={(e) => {
                setResultSecondsInput(e.currentTarget.value);
                setResultSecondsDirty(true);
                setSettingsError(undefined);
              }}
              onChange={handleSaveSettings}
            />
            <button
              type="button"
              name="load_result_seconds"
              disabled={savingSettings()}
              onClick={() => {
                setResultSecondsDirty(false);
                void handleReadSettings();
              }}
            >
              {lang() === 'en' ? 'Read' : 'อ่านค่า'}
            </button>
            <button
              type="button"
              name="save_result_seconds"
              class="primary"
              disabled={!known() || savingSettings()}
              onClick={handleSaveSettings}
            >
              {lang() === 'en' ? 'Save' : 'บันทึก'}
            </button>
            <Show when={settingsError()}>
              {(err) => (
                <span role="alert" class="warning" data-error="validationFailed">
                  {err()}
                </span>
              )}
            </Show>
          </div>
        </div>
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
              onToggle={handleToggle}
            />
          )}
        </For>
      </div>
    </div>
  );
}
