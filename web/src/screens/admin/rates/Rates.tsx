import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type {
  RateDraftDetailData,
  RateDraftFilter,
} from '../../../shared/api/generated/rate_draft';
import type { RateDraftsData } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatePolicySaveInput, RatePolicySaveOutput } from '../../../shared/api/generated/rate_policy_save';
import type {
  RatesReferenceData,
  RatesReferenceFilter,
  RatesReferenceRow,
} from '../../../shared/api/generated/rates_reference';
import { RatesHistory } from './RatesHistory';
import type { RatePublicationRow } from './rate-history';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { PermissionsUnavailable, permissionsUnavailableText } from '../../../shared/ui/PermissionsUnavailable';
import { RateFreshness } from '../../../shared/ui/RateFreshness';
import { formatRate } from '../../../shared/ui/rate';
import type { AdminMenuProps } from '../menus';
import { openDialog } from '../parts/Dialog';
import { toast } from '../parts/toast';
import * as table from './i18n';
import { emptyValueTiers, RatesPolicyCard, type TierFormRow } from './RatesPolicyCard';
import { RatesDraftCard, type OutOfBoundItem } from './RatesDraftCard';
import './rates.css';

const t = screenT('admin.rates', table);

function formatTime(iso: string): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

export function Rates(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();
  const writes = new Map<string, { send(body: unknown): Promise<unknown>; reset(): void }>();

  function writeFor<B, R>(actionId: string, method: string, path: string): { send(body: B): Promise<R>; reset(): void } {
    let w = writes.get(actionId);
    if (!w) {
      w = keyedWrite<unknown, unknown>(client, method, path);
      writes.set(actionId, w);
    }
    return w as { send(body: B): Promise<R>; reset(): void };
  }

  const savePolicyWrite = keyedWrite<RatePolicySaveInput & { branch: string }, RatePolicySaveOutput>(
    client,
    'POST',
    '/api/rate-policies',
  );

  const [pubData, setPubData] = createSignal<PublishedRatesData | null>(null);
  const [pubError, setPubError] = createSignal<string | null>(null);
  const [, setDraftsData] = createSignal<RateDraftsData | null>(null);
  const [draftsError, setDraftsError] = createSignal<string | null>(null);
  const [draftDetail, setDraftDetail] = createSignal<RateDraftDetailData | null>(null);
  const [policyData, setPolicyData] = createSignal<RatePolicyData | null>(null);
  const [policyError, setPolicyError] = createSignal<string | null>(null);

  const [permissions, setPermissions] = createSignal<readonly string[]>([]);
  const [meError, setMeError] = createSignal<unknown>(null);
  const [meBusy, setMeBusy] = createSignal<boolean>(false);
  const canSavePolicy = () => permissions().includes('RatePolicySave');

  const [valueTiersOn, setValueTiersOn] = createSignal<boolean>(false);
  const [valueTiers, setValueTiers] = createSignal<TierFormRow[]>(emptyValueTiers());
  const [sharpMoveAlertPct, setSharpMoveAlertPct] = createSignal<string>('');

  const [refData, setRefData] = createSignal<RatesReferenceData | null>(null);
  const [refVersion, setRefVersion] = createSignal<number | undefined>(undefined);
  const [refError, setRefError] = createSignal<string | null>(null);
  const [refLoading, setRefLoading] = createSignal<boolean>(true);

  const [simulateResult, setSimulateResult] = createSignal<{
    id: string;
    isInBound: boolean;
    outOfBound: OutOfBoundItem[];
  } | null>(null);

  const [adjustBuyPct, setAdjustBuyPct] = createSignal<string>('0.0000');
  const [adjustSellPct, setAdjustSellPct] = createSignal<string>('0.0000');

  // #2410: one write at a time. A second click while a write is in flight would
  // resend the same idempotency key, get 409 idempotencyConflict, and lose the key.
  const [busy, setBusy] = createSignal<boolean>(false);
  const guarded = (fn: () => Promise<void>) => async (): Promise<void> => {
    if (busy()) return;
    setBusy(true);
    try {
      await fn();
    } finally {
      setBusy(false);
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
      } else if (!res.changed) {
        setRefError(refData()?.error ?? null);
      }
    } catch (err: unknown) {
      setRefError(wordedError(err));
    } finally {
      setRefLoading(false);
    }
  };

  let readGen = 0;

  const readPublished = async (branch: string, gen: number) => {
    try {
      const res = await client.view<PublishedRatesData>('published_rates', { branch });
      if (gen !== readGen) return;
      if (res.changed && res.data) {
        setPubData(res.data);
      }
      setPubError(null);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      setPubData(null);
      setPubError(wordedError(err));
    }
  };

  const readDrafts = async (branch: string, gen: number) => {
    try {
      const res = await client.view<RateDraftsData>('rate_drafts', { branch });
      if (gen !== readGen) return;
      if (res.changed && res.data) {
        setDraftsData(res.data);
      }
      const draftsList = res.data?.drafts ?? [];
      const openDraft = draftsList.find((d) =>
        ['DRAFT', 'PENDING_APPROVAL', 'APPROVED'].includes(d.status),
      );
      if (openDraft) {
        const detailRes = await client.view<RateDraftDetailData, RateDraftFilter>(
          'rate_draft',
          { branch, filter: { id: openDraft.id } },
        );
        if (gen !== readGen) return;
        if (detailRes.changed && detailRes.data) {
          setDraftDetail(detailRes.data);
        }
      } else {
        setDraftDetail(null);
      }
      setDraftsError(null);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      setDraftsData(null);
      setDraftDetail(null);
      setDraftsError(wordedError(err));
    }
  };

  const readMe = async (gen: number) => {
    setMeBusy(true);
    try {
      const res = await client.view<{ permissions?: readonly string[] }>('me');
      if (gen !== readGen) return;
      if (res.data && Array.isArray(res.data.permissions)) {
        setPermissions(res.data.permissions);
      } else {
        setPermissions([]);
      }
      setMeError(null);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      setPermissions([]);
      setMeError(err);
    } finally {
      if (gen === readGen) {
        setMeBusy(false);
      }
    }
  };

  const readPolicy = async (branch: string, gen: number) => {
    try {
      const res = await client.view<RatePolicyData>('rate_policy', { branch });
      if (gen !== readGen) return;
      if (res.changed && res.data) {
        setPolicyData(res.data);
        if (res.data.policy) {
          setAdjustBuyPct(res.data.policy.buyAdjustPct);
          setAdjustSellPct(res.data.policy.sellAdjustPct);
          setValueTiersOn(Boolean(res.data.policy.valueTiersOn));
          setSharpMoveAlertPct(res.data.policy.sharpMoveAlertPct ?? '');
          const incoming = res.data.policy.valueTiers ?? [];
          setValueTiers(
            ([1, 2, 3, 4] as const).map((tierNo) => {
              const found = incoming.find((item) => item.tierNo === tierNo);
              return {
                tierNo,
                upperInclusiveThb: tierNo === 4 ? '' : (found?.upperInclusiveThb ?? ''),
                buyMarginPct: found?.buyMarginPct ?? '',
                sellMarginPct: found?.sellMarginPct ?? '',
              };
            }),
          );
          savePolicyWrite.reset();
        } else {
          setValueTiersOn(false);
          setValueTiers(emptyValueTiers());
          setSharpMoveAlertPct('');
          savePolicyWrite.reset();
        }
      }
      setPolicyError(null);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      setPolicyData(null);
      setPolicyError(wordedError(err));
    }
  };

  const readAll = async (branch: string) => {
    if (!props.shown()) return;
    const gen = ++readGen;
    setSimulateResult(null);
    void fetchReference({ since: undefined });
    await Promise.all([readMe(gen), readPublished(branch, gen), readDrafts(branch, gen), readPolicy(branch, gen)]);
  };

  let lastShown = false;
  let lastBranch = '';

  createEffect(() => {
    const shown = props.shown();
    const branch = props.branch();

    if (!shown) {
      lastShown = false;
      return;
    }

    if (!lastShown) {
      lastShown = true;
      lastBranch = branch;
      void readAll(branch);
      return;
    }

    if (branch !== lastBranch) {
      lastBranch = branch;
      setPubData(null);
      setDraftsData(null);
      setDraftDetail(null);
      setPolicyData(null);
      setValueTiersOn(false);
      setValueTiers(emptyValueTiers());
      setSharpMoveAlertPct('');
      savePolicyWrite.reset();
      void readAll(branch);
      return;
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

  let lastDraftId: string | undefined;
  createEffect(() => {
    const curDraftId = draftDetail()?.draft?.id;
    if (curDraftId !== lastDraftId) {
      lastDraftId = curDraftId;
      setSimulateResult(null);
    }
  });

  const openHistory = async () => {
    const branch = props.branch();
    const gen = readGen;
    try {
      const rows = await client.page<RatePublicationRow>('rate_publications', { branch, limit: 20 });
      if (gen !== readGen) return;
      openDialog(
        t('historyTitle'),
        () => (
          <div class="rate-history-dialog">
            <RatesHistory client={client} branch={branch} first={rows} />
          </div>
        ),
      );
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  return (
    <div class="admin-rates-screen">
      {/* R5 Kiosk Currency Link */}
      <div class="rate-tools">
        <p>{t('kioskCurrencyNote')}</p>
        <a href={`/kiosk-currencies.html?branch=${props.branch()}`}>
          <span aria-hidden="true">◎</span> {t('kioskCurrencyLink')}
        </a>
      </div>

      {/* Rate Policy: Value Tiers & Sharp-Move Threshold (VT-5) */}
      <PermissionsUnavailable
        error={meError()}
        onRetry={() => void readMe(readGen)}
        busy={meBusy()}
      />
      <RatesPolicyCard
        branch={props.branch}
        busy={busy}
        canSavePolicy={canSavePolicy}
        disabledReason={meError() !== null ? permissionsUnavailableText(meError()) : undefined}
        policyData={policyData}
        policyError={policyError}
        valueTiers={valueTiers}
        setValueTiers={setValueTiers}
        valueTiersOn={valueTiersOn}
        setValueTiersOn={setValueTiersOn}
        sharpMoveAlertPct={sharpMoveAlertPct}
        setSharpMoveAlertPct={setSharpMoveAlertPct}
        adjustBuyPct={adjustBuyPct}
        adjustSellPct={adjustSellPct}
        savePolicyWrite={savePolicyWrite}
        readGen={() => readGen}
        setSimulateResult={setSimulateResult}
        guarded={guarded}
        toast={toast}
        t={t}
      />

      {/* Draft Section */}
      <RatesDraftCard
        branch={props.branch}
        busy={busy}
        draftDetail={draftDetail}
        setDraftDetail={setDraftDetail}
        simulateResult={simulateResult}
        setSimulateResult={setSimulateResult}
        draftsError={draftsError}
        policyError={policyError}
        policyData={policyData}
        pubData={pubData}
        refData={refData}
        adjustBuyPct={adjustBuyPct}
        setAdjustBuyPct={setAdjustBuyPct}
        adjustSellPct={adjustSellPct}
        setAdjustSellPct={setAdjustSellPct}
        client={client}
        writeFor={writeFor}
        readDrafts={readDrafts}
        readPublished={readPublished}
        readGen={() => readGen}
        guarded={guarded}
        toast={toast}
        t={t}
      />

      {/* Published Rates Section */}
      <div class="rates-section">
        <div class="section-header">
          <div class="title-row">
            <div>
              <div class="eyebrow">{t('ratesEyebrow')}</div>
              <h2>{t('ratesTitle')}</h2>
            </div>
            <div class="rates-pill">{t('ratesPill')}</div>
          </div>
          <Show when={pubError()}>
            <div class="form-error" role="alert">{pubError()}</div>
          </Show>
        </div>

        <RateFreshness
          stale={pubData()?.stale ?? false}
          sourceDate={pubData()?.sourceDate}
          publishedAt={pubData()?.publication?.publishedAt}
        />

        <div class="table-scroll">
          <table class="rates-table">
            <thead>
              <tr>
                <th>{t('headerCurrency')}</th>
                <th class="num">{t('headerBuy')}</th>
                <th class="num">{t('headerSell')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={pubData()?.lines ?? []}>
                {(l) => (
                  <tr data-published-line={l.currency}>
                    <td class="code">{l.currency}</td>
                    <td class="num rate-number">{l.buyRate !== null ? formatRate(l.buyRate, l.unitSize, l.currency) : t('notSet')}</td>
                    <td class="num rate-number">{l.sellRate !== null ? formatRate(l.sellRate, l.unitSize, l.currency) : t('notSet')}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </div>

      {/* BOT Reference Section */}
      <div class="bot-reference-section">
        <div class="section-header">
          <div class="title-row">
            <div>
              <div class="eyebrow">{t('botEyebrow')}</div>
              <h2>{t('botTitle')}</h2>
              <div class="subtitle">{t('botSubtitle')}</div>
              <div class="status-row">
                <Show when={refLoading()}>
                  <p class="loading">{t('botLoading')}</p>
                </Show>
                <Show when={!refLoading() && refData()?.fetchedAt}>
                  <p class="fetched">
                    {t('botLastFetchedPrefix')}{formatTime(refData()!.fetchedAt)}
                  </p>
                </Show>
                <Show when={refError()}>
                  <p class="bot-error" role="alert">{refError()}</p>
                </Show>
              </div>
            </div>
            <div class="dialog-actions">
              <button
                type="button"
                class="secondary"
                onClick={() => fetchReference({ fresh: true })}
              >
                {t('botRefreshButton')}
              </button>
              <button
                type="button"
                class="secondary"
                onClick={openHistory}
              >
                {t('rateHistoryButton')}
              </button>
            </div>
          </div>
        </div>

        <Show when={refData()?.rows && refData()!.rows.length > 0}>
          <div class="table-scroll">
            <table class="comparison-table">
              <thead>
                <tr>
                  <th>{t('headerPair')}</th>
                  <th>{t('headerBotRates')}</th>
                </tr>
              </thead>
              <tbody>
                <For each={refData()!.rows}>
                  {(r: RatesReferenceRow) => (
                    <tr>
                      <td><strong>{r.code}</strong></td>
                      <td>
                        {formatRate(r.buyingSight, r.unit, r.code)} / {formatRate(r.buyingTransfer, r.unit, r.code)} / {formatRate(r.selling, r.unit, r.code)}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>
    </div>
  );
}
