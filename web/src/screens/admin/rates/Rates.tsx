import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminBranchItem, AdminBranchesData } from '../../../shared/api/generated/admin_branches';
import type { AdminRateHistoryRow } from '../../../shared/api/generated/admin_rate_history';
import type { AdminRateRow, AdminRatesData } from '../../../shared/api/generated/admin_rates';
import type {
  RatesReferenceData,
  RatesReferenceFilter,
  RatesReferenceRow,
} from '../../../shared/api/generated/rates_reference';
import { locale, screenT } from '../../../shared/i18n';
import type { AdminMenuProps } from '../menus';
import { DataTable } from '../parts/DataTable';
import { closeDialog, openDialog } from '../parts/Dialog';
import { pendingWrite } from '../parts/pending-write';
import { refreshTick } from '../parts/refresh';
import * as table from './i18n';
import './rates.css';

const t = screenT('admin.rates', table);

function formatTime(iso: string): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function renderBotLine(r: RatesReferenceRow): string {
  return `${t('botDatePrefix')}${r.date} · ${t('botSight')} ${r.buyingSight} · ${t('botTransfer')} ${r.buyingTransfer} · ${t('botSelling')} ${r.selling}`;
}

export function Rates(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();

  const [ratesData, setRatesData] = createSignal<AdminRatesData | null>(null);
  const [ratesVersion, setRatesVersion] = createSignal<number | undefined>(undefined);

  const [refData, setRefData] = createSignal<RatesReferenceData | null>(null);
  const [refVersion, setRefVersion] = createSignal<number | undefined>(undefined);
  const [refError, setRefError] = createSignal<string | null>(null);
  const [refLoading, setRefLoading] = createSignal<boolean>(true);

  const [branches, setBranches] = createSignal<AdminBranchItem[]>([]);

  const readRates = async (opts: { since?: number }) => {
    if (!props.shown()) return;
    const branch = props.branch();
    try {
      const res = await client.view<AdminRatesData>('admin_rates', {
        branch,
        since: opts.since,
      });
      setRatesVersion(res.version);
      if (res.changed && res.data) {
        setRatesData(res.data);
      }
    } catch {
      // Rates error handling
    }
  };

  const fetchReference = async (opts: { since?: number; fresh?: boolean }) => {
    if (!props.shown()) return;
    try {
      const res = await client.view<RatesReferenceData, RatesReferenceFilter>('rates_reference', {
        since: opts.since,
        filter: opts.fresh ? { fresh: '1' } : undefined,
      });
      setRefVersion(res.version);
      if (res.changed && res.data) {
        setRefData(res.data);
        setRefError(res.data.error ?? null);
      }
    } catch (err: unknown) {
      if (err instanceof ApiError) {
        setRefError(err.message);
      } else if (err && typeof err === 'object' && 'message' in err) {
        setRefError(String((err as { message: unknown }).message));
      } else {
        setRefError(String(err));
      }
    } finally {
      setRefLoading(false);
    }
  };

  let lastShown = false;
  let lastBranch = '';
  let lastTick = refreshTick();

  createEffect(() => {
    const shown = props.shown();
    const branch = props.branch();
    const tick = refreshTick();

    if (!shown) {
      lastShown = false;
      return;
    }

    if (!lastShown) {
      lastShown = true;
      lastBranch = branch;
      lastTick = tick;
      void readRates({ since: undefined });
      void fetchReference({ since: undefined, fresh: false });
      return;
    }

    if (branch !== lastBranch) {
      lastBranch = branch;
      lastTick = tick;
      void readRates({ since: undefined });
      return;
    }

    if (tick !== lastTick) {
      lastTick = tick;
      void readRates({ since: ratesVersion() });
    }
  });

  let pollInterval: ReturnType<typeof setInterval> | undefined;

  const poll = async () => {
    if (!props.shown() || document.hidden) return;
    await fetchReference({ since: refVersion() });
  };

  onMount(() => {
    pollInterval = setInterval(poll, 60_000);
  });

  onCleanup(() => {
    if (pollInterval !== undefined) {
      clearInterval(pollInterval);
    }
  });

  const openEdit = (rate: AdminRateRow) => {
    void fetchReference({ since: undefined, fresh: false });

    const [buyVal, setBuyVal] = createSignal(rate.buy ?? '');
    const [sellVal, setSellVal] = createSignal(rate.sell ?? '');
    const [partnerNameVal, setPartnerNameVal] = createSignal(rate.partnerName ?? '');
    const [partnerCostVal, setPartnerCostVal] = createSignal(rate.partnerCost ?? '');
    const [formError, setFormError] = createSignal<string | null>(null);

    const handleSave = async () => {
      const trimmedName = partnerNameVal().trim();
      const trimmedCost = partnerCostVal().trim();
      if ((trimmedName === '') !== (trimmedCost === '')) {
        setFormError(t('partnerPairError'));
        return;
      }
      setFormError(null);
      await pendingWrite.run(
        client,
        props.branch(),
        'adminSaveRate',
        {
          code: rate.code,
          buy: buyVal(),
          sell: sellVal(),
          partnerName: trimmedName,
          partnerCost: trimmedCost === '' ? null : trimmedCost,
          previousUpdated: rate.updated,
        },
        () => {
          void readRates({ since: ratesVersion() });
        },
      );
    };

    openDialog(`${t('editTitlePrefix')}${rate.code}`, () => {
      const currentRef = refData();
      const err = refError();
      const botRow = currentRef?.rows.find((r) => r.code === rate.code);

      return (
        <div class="edit-rate-dialog">
          <p class="notice">{t('editNotice')}</p>
          <div class="bot-reference-info">
            <Show when={err}>
              <p class="bot-error">{err}</p>
            </Show>
            <Show when={!err && botRow}>
              {(br) => (
                <p class="bot-line">
                  {renderBotLine(br())}
                  <Show when={br().unit === 100 || rate.code === 'JPY'}>
                    <span>{t('jpyConversionNote')}</span>
                  </Show>
                  <br />
                  <span>
                    {t('botFetchedPrefix')}
                    {formatTime(currentRef!.fetchedAt)}
                  </span>
                </p>
              )}
            </Show>
            <Show when={!err && !botRow}>
              <p class="no-bot">{t('noReferenceData')}</p>
            </Show>
          </div>
          <p class="save-notice">{t('botSaveNotice')}</p>

          <form
            onSubmit={(e) => {
              e.preventDefault();
              void handleSave();
            }}
          >
            <div class="field">
              <label>
                <span>{t('headerBuy')}</span>
                <input
                  type="number"
                  required
                  min="0.0001"
                  max="1000000"
                  step="0.0001"
                  value={buyVal()}
                  onInput={(e) => setBuyVal(e.currentTarget.value)}
                />
              </label>
            </div>

            <div class="field">
              <label>
                <span>{t('headerSell')}</span>
                <input
                  type="number"
                  required
                  min="0.0001"
                  max="1000000"
                  step="0.0001"
                  value={sellVal()}
                  onInput={(e) => setSellVal(e.currentTarget.value)}
                />
              </label>
            </div>

            <div class="field">
              <label>
                <span>{t('partnerNameLabel')}</span>
                <input
                  type="text"
                  maxLength={100}
                  placeholder={t('partnerNamePlaceholder')}
                  value={partnerNameVal()}
                  onInput={(e) => setPartnerNameVal(e.currentTarget.value)}
                />
              </label>
            </div>

            <div class="field">
              <label>
                <span>{`${t('partnerCostPrefix')}${rate.code})`}</span>
                <input
                  type="number"
                  min="0.0001"
                  max="1000000"
                  step="0.0001"
                  value={partnerCostVal()}
                  onInput={(e) => setPartnerCostVal(e.currentTarget.value)}
                />
              </label>
            </div>

            <Show when={formError()}>
              <div class="form-error">{formError()}</div>
            </Show>

            <div class="dialog-actions">
              <button type="button" onClick={closeDialog}>
                {t('cancel')}
              </button>
              <button type="submit">{t('saveToServer')}</button>
            </div>
          </form>
        </div>
      );
    });
  };

  const openHistory = async () => {
    let branchName = props.branch();
    if (branches().length === 0) {
      try {
        const bRes = await client.view<AdminBranchesData>('admin_branches');
        if (bRes.data?.branches) {
          setBranches(bRes.data.branches);
          const found = bRes.data.branches.find((b) => b.id === props.branch());
          if (found) branchName = found.name;
        }
      } catch {
        // Fallback to branch id
      }
    } else {
      const found = branches().find((b) => b.id === props.branch());
      if (found) branchName = found.name;
    }

    const allRows: AdminRateHistoryRow[] = [];
    let beforeId: string | undefined = undefined;

    while (true) {
      const pageRows: AdminRateHistoryRow[] = await client.page<AdminRateHistoryRow>('admin_rate_history', {
        branch: props.branch(),
        before: beforeId,
        limit: 100,
      });
      allRows.push(...pageRows);
      if (pageRows.length < 100) {
        break;
      }
      beforeId = pageRows[pageRows.length - 1].id;
    }

    openDialog(`${t('historyTitlePrefix')}${branchName}`, () => (
      <div class="rate-history-dialog">
        <p class="history-note">{t('historyNote')}</p>
        <DataTable
          headers={[
            { text: t('headerTimeActor') },
            { text: t('headerPairShopRate') },
            { text: t('headerPartnerCostHistory') },
            { text: t('headerBotReferenceCopy') },
          ]}
          rowCount={allRows.length}
        >
          {allRows.map((row) => {
            const botRow = row.botReference?.rows.find((r) => r.code === row.code);
            return (
              <tr>
                <td>
                  <div>{formatTime(row.updated)}</div>
                  <div>{row.actor}</div>
                </td>
                <td>
                  <div>{row.code}/THB</div>
                  <div>
                    {row.buy} / {row.sell}
                  </div>
                </td>
                <td>
                  <Show when={row.partnerName || row.partnerCost}>
                    <div>{row.partnerName || '—'}</div>
                    <div>{row.partnerCost || '—'}</div>
                  </Show>
                  <Show when={!row.partnerName && !row.partnerCost}>
                    <div>— —</div>
                  </Show>
                </td>
                <td>
                  <Show when={botRow}>
                    <div>
                      {renderBotLine(botRow!)}
                      <Show when={botRow!.unit === 100 || row.code === 'JPY'}>
                        <span>{t('jpyConversionNote')}</span>
                      </Show>
                    </div>
                    <div>
                      {t('botFetchedPrefix')}
                      {formatTime(row.botReference!.fetchedAt)}
                    </div>
                  </Show>
                  <Show when={!botRow}>
                    <div>{t('noReferenceData')}</div>
                  </Show>
                </td>
              </tr>
            );
          })}
        </DataTable>
      </div>
    ));
  };

  return (
    <div class="admin-rates-screen">
      <div class="kiosk-currency-link-bar">
        <p>{t('kioskCurrencyNote')}</p>
        <a href={`/kiosk-currencies.html?branch=${props.branch()}`}>
          <span aria-hidden="true">◎</span>
          <span>{t('kioskCurrencyLink')}</span>
        </a>
      </div>

      <section class="rates-section">
        <header class="section-header">
          <div class="eyebrow">{t('ratesEyebrow')}</div>
          <div class="title-row">
            <h2>{t('ratesTitle')}</h2>
            <span class="pill">{t('ratesPill')}</span>
          </div>
          <p class="live-note">{t('ratesLiveNote')}</p>
          <Show when={ratesData()?.updated}>
            <p class="last-edited">
              {t('lastEditedPrefix')}
              {formatTime(ratesData()!.updated)}
            </p>
          </Show>
        </header>

        <table class="rates-table">
          <thead>
            <tr>
              <th>{t('headerCurrency')}</th>
              <th>{t('headerBuy')}</th>
              <th>{t('headerSell')}</th>
              <th>{t('headerSpread')}</th>
              <th />
            </tr>
          </thead>
          <tbody>
            {(ratesData()?.rates ?? []).map((rate) => (
              <tr>
                <td>
                  <span aria-hidden="true">{rate.symbol}</span>
                  <span class="code">{rate.code}</span>
                  <span class="name">{rate.name}</span>
                </td>
                <td>{rate.buy ?? t('notSet')}</td>
                <td>{rate.sell ?? t('notSet')}</td>
                <td>{rate.spread ?? '—'}</td>
                <td>
                  <button
                    type="button"
                    aria-label={`${t('editRateAria')}${rate.code}`}
                    onClick={() => openEdit(rate)}
                  >
                    {t('editRateButton')}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section class="bot-reference-section">
        <header class="section-header">
          <div class="eyebrow">{t('botEyebrow')}</div>
          <h2>{t('botTitle')}</h2>
          <p class="subtitle">{t('botSubtitle')}</p>
          <div class="status-row">
            <Show when={refLoading() && !refError()}>
              <p class="loading">{t('botLoading')}</p>
            </Show>
            <Show when={refError()}>
              <p class="error">{refError()}</p>
            </Show>
            <Show when={!refLoading() && !refError() && refData()?.fetchedAt}>
              <p class="fetched">
                {t('botLastFetchedPrefix')}
                {formatTime(refData()!.fetchedAt)}
              </p>
            </Show>
          </div>
          <div class="actions">
            <button type="button" onClick={() => fetchReference({ fresh: true })}>
              {t('botRefreshButton')}
            </button>
            <button type="button" onClick={openHistory}>
              {t('rateHistoryButton')}
            </button>
            <a
              href="https://www.bot.or.th/th/statistics/exchange-rate.html"
              target="_blank"
              rel="noopener"
            >
              {t('botSourceLink')}
            </a>
          </div>
        </header>

        <table class="comparison-table">
          <thead>
            <tr>
              <th>{t('headerPair')}</th>
              <th>{t('headerShopRates')}</th>
              <th>{t('headerBotRates')}</th>
              <th>{t('headerPartnerCost')}</th>
              <th>{t('headerPartnerSpread')}</th>
            </tr>
          </thead>
          <tbody>
            {(ratesData()?.rates ?? []).map((rate) => {
              const botRow = !refError() ? refData()?.rows.find((r) => r.code === rate.code) : undefined;
              return (
                <tr>
                  <td>{rate.code}/THB</td>
                  <td>
                    {rate.buy ?? '—'} / {rate.sell ?? '—'}
                  </td>
                  <td>
                    <Show when={botRow}>
                      {(br) => (
                        <div>
                          <div>
                            {br().buyingSight} / {br().buyingTransfer} / {br().selling}
                          </div>
                          <div>{br().date}</div>
                        </div>
                      )}
                    </Show>
                    <Show when={!botRow}>—</Show>
                  </td>
                  <td>
                    <Show when={rate.partnerCost}>
                      <div>{rate.partnerCost}</div>
                      <div>{rate.partnerName}</div>
                    </Show>
                    <Show when={!rate.partnerCost}>
                      <div>—</div>
                      <div>{t('partnerNotSpecified')}</div>
                    </Show>
                  </td>
                  <td>{rate.partnerSpread ?? '—'}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </section>
    </div>
  );
}
