import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type {
  RateDraftDetailData,
  RateDraftFilter,
  RateDraftLineDto,
} from '../../../shared/api/generated/rate_draft';
import type { RateDraftsData } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatePublicationsData } from '../../../shared/api/generated/rate_publications';
import type {
  RatesReferenceData,
  RatesReferenceFilter,
  RatesReferenceRow,
} from '../../../shared/api/generated/rates_reference';
import { locale, screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { RateFreshness } from '../../../shared/ui/RateFreshness';
import { formatRate } from '../../../shared/ui/rate';
import type { AdminMenuProps } from '../menus';
import { openDialog } from '../parts/Dialog';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './rates.css';

const t = screenT('admin.rates', table);

function formatTime(iso: string): string {
  if (!iso) return '';
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

interface OutOfBoundItem {
  currency: string;
  side: 'BUY' | 'SELL';
}

export function Rates(props: AdminMenuProps): JSX.Element {
  const client = new ApiClient();

  const [pubData, setPubData] = createSignal<PublishedRatesData | null>(null);
  const [pubError, setPubError] = createSignal<string | null>(null);
  const [, setDraftsData] = createSignal<RateDraftsData | null>(null);
  const [draftsError, setDraftsError] = createSignal<string | null>(null);
  const [draftDetail, setDraftDetail] = createSignal<RateDraftDetailData | null>(null);
  const [policyData, setPolicyData] = createSignal<RatePolicyData | null>(null);
  const [policyError, setPolicyError] = createSignal<string | null>(null);

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
      setRefError(wordedError(err));
    } finally {
      setRefLoading(false);
    }
  };

  const readPublished = async (branch: string) => {
    try {
      const res = await client.view<PublishedRatesData>('published_rates', { branch });
      if (res.changed && res.data) {
        setPubData(res.data);
      }
      setPubError(null);
    } catch (err: unknown) {
      setPubData(null);
      setPubError(wordedError(err));
    }
  };

  const readDrafts = async (branch: string) => {
    try {
      const res = await client.view<RateDraftsData>('rate_drafts', { branch });
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
        if (detailRes.changed && detailRes.data) {
          setDraftDetail(detailRes.data);
        }
      } else {
        setDraftDetail(null);
      }
      setDraftsError(null);
    } catch (err: unknown) {
      setDraftsData(null);
      setDraftDetail(null);
      setDraftsError(wordedError(err));
    }
  };

  const readPolicy = async (branch: string) => {
    try {
      const res = await client.view<RatePolicyData>('rate_policy', { branch });
      if (res.changed && res.data) {
        setPolicyData(res.data);
        if (res.data.policy) {
          setAdjustBuyPct(res.data.policy.buyAdjustPct);
          setAdjustSellPct(res.data.policy.sellAdjustPct);
        }
      }
      setPolicyError(null);
    } catch (err: unknown) {
      setPolicyData(null);
      setPolicyError(wordedError(err));
    }
  };

  const readAll = async (branch: string) => {
    if (!props.shown()) return;
    setSimulateResult(null);
    void fetchReference({ since: undefined });
    await Promise.all([readPublished(branch), readDrafts(branch), readPolicy(branch)]);
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

  const getUnitSize = (currency: string): string => {
    const pubLine = pubData()?.lines?.find((p) => p.currency === currency);
    if (pubLine?.unitSize) return pubLine.unitSize;
    const rRow = refData()?.rows?.find((r) => r.code === currency);
    if (rRow?.unit) return String(rRow.unit);
    return '1';
  };

  const isOutOfBound = (currency: string): boolean => {
    const sim = simulateResult();
    if (!sim || !Array.isArray(sim.outOfBound)) return false;
    return sim.outOfBound.some((item) => item.currency === currency);
  };

  const formatStatus = (status: string | undefined): string => {
    if (!status) return '';
    switch (status) {
      case 'DRAFT':
        return t('statusDraft');
      case 'PENDING_APPROVAL':
        return t('statusPendingApproval');
      case 'APPROVED':
        return t('statusApproved');
      case 'PUBLISHED':
        return t('statusPublished');
      case 'REJECTED':
        return t('statusRejected');
      default:
        return status;
    }
  };

  const handleNewDraft = async () => {
    const branch = props.branch();
    try {
      await client.command(
        'POST',
        '/api/rate-references/fetch',
        { branch },
        crypto.randomUUID(),
      );

      const curPub = pubData();
      const lines = (curPub?.lines ?? []).map((l: PublishedLineDto) => ({
        currency: l.currency,
        buyMode: 'AUTO',
        sellMode: 'AUTO',
        manualBuy: null,
        manualSell: null,
        reason: null,
      }));

      const createRes = await client.command<unknown, { id: string; version: number; status: string }>(
        'POST',
        '/api/rate-drafts',
        { branch, lines },
        crypto.randomUUID(),
      );

      const detailRes = await client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: createRes.id } },
      );
      if (detailRes.data) {
        setDraftDetail(detailRes.data);
      }

      await readDrafts(branch);
      setSimulateResult(null);
    } catch (err: unknown) {
      toast(wordedError(err));
    }
  };

  const handleSimulate = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    try {
      const res = await client.command<
        unknown,
        {
          id: string;
          isInBound: boolean;
          outOfBound: OutOfBoundItem[];
        }
      >(
        'POST',
        `/api/rate-drafts/${draft.id}/simulate`,
        { branch },
        crypto.randomUUID(),
      );
      setSimulateResult(res);
    } catch (err: unknown) {
      toast(wordedError(err));
    }
  };

  const handleSubmit = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    try {
      const res = await client.command<unknown, { requestId?: string; status: string }>(
        'POST',
        `/api/rate-drafts/${draft.id}/submit`,
        { branch },
        crypto.randomUUID(),
      );
      if (res?.status) {
        setDraftDetail((prev) =>
          prev && prev.draft ? { ...prev, draft: { ...prev.draft, status: res.status } } : prev,
        );
      }
    } catch (err: unknown) {
      toast(wordedError(err));
    }
  };

  const handlePublish = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const curVersion = pubData()?.publication?.version ?? null;
    try {
      await client.command<unknown, { id: string; version: number }>(
        'POST',
        `/api/rate-drafts/${draft.id}/publish`,
        { branch, expectedVersion: curVersion },
        crypto.randomUUID(),
      );
      await readPublished(branch);
      await readDrafts(branch);
    } catch (err: unknown) {
      toast(wordedError(err));
    }
  };

  const handleAdjustAll = async () => {
    const draft = draftDetail()?.draft;
    const lines = draftDetail()?.lines ?? [];
    if (!draft) return;
    const branch = props.branch();
    const policy = policyData()?.policy;
    if (!policy) return;
    try {
      await client.command(
        'POST',
        '/api/rate-policies',
        {
          branch,
          buyAdjustPct: adjustBuyPct(),
          sellAdjustPct: adjustSellPct(),
          guards: policy.guards,
          maxReferenceAgeHours: policy.maxReferenceAgeHours,
          onStale: policy.onStale,
          staleLimitThb: policy.staleLimitThb,
          reason: policy.reason || t('adjustAll'),
        },
        crypto.randomUUID(),
      );

      const putLines = lines.map((l: RateDraftLineDto) => ({
        currency: l.currency,
        buyMode: l.buyMode,
        sellMode: l.sellMode,
        manualBuy: l.manualBuy,
        manualSell: l.manualSell,
        reason: l.reason,
      }));

      await client.command(
        'PUT',
        `/api/rate-drafts/${draft.id}`,
        {
          branch,
          expectedVersion: draft.version,
          lines: putLines,
        },
        crypto.randomUUID(),
      );

      const updated = await client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: draft.id } },
      );
      if (updated.data) {
        setDraftDetail(updated.data);
      }
    } catch (err: unknown) {
      toast(wordedError(err));
    }
  };

  const openHistory = async () => {
    const branch = props.branch();
    try {
      const res = await client.view<RatePublicationsData>('rate_publications', { branch });
      const pubs = res.data?.publications ?? [];
      openDialog(
        t('historyTitle'),
        () => (
          <div class="rate-history-dialog">
            <div class="table-scroll">
              <table class="rates-table">
                <thead>
                  <tr>
                    <th>{t('headerVersion')}</th>
                    <th>{t('headerPublishedBy')}</th>
                    <th>{t('headerPublishedAt')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={pubs}>
                    {(p) => (
                      <tr>
                        <td>{p.version}</td>
                        <td>{p.publishedBy}</td>
                        <td>{formatTime(p.publishedAt)}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </div>
        ),
      );
    } catch (err: unknown) {
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

      {/* Draft Section */}
      <div class="rates-section">
        <div class="section-header">
          <div class="title-row">
            <div>
              <div class="eyebrow">{t('eyebrow')}</div>
              <h2>{t('draftTitle')}</h2>
              <Show when={draftDetail()?.draft}>
                <div class="live-note">
                  <span class="draft-status-label">
                    {formatStatus(draftDetail()?.draft?.status)}
                  </span>
                </div>
              </Show>
            </div>
            <div class="dialog-actions">
              <button
                type="button"
                class="btn-new-draft"
                onClick={handleNewDraft}
              >
                {t('newDraft')}
              </button>
              <Show when={draftDetail()?.draft}>
                <button
                  type="button"
                  class="btn-simulate"
                  onClick={handleSimulate}
                >
                  {t('simulate')}
                </button>
                <button
                  type="button"
                  class="btn-submit"
                  onClick={handleSubmit}
                >
                  {t('submit')}
                </button>
                <button
                  type="button"
                  class="btn-publish"
                  onClick={handlePublish}
                >
                  {t('publish')}
                </button>
              </Show>
            </div>
          </div>
          <Show when={simulateResult() && simulateResult()!.isInBound}>
            <div class="in-bound-status">{t('inBound')}</div>
          </Show>
          <Show when={draftsError()}>
            <div class="form-error" role="alert">{draftsError()}</div>
          </Show>
          <Show when={policyError()}>
            <div class="form-error" role="alert">{policyError()}</div>
          </Show>
        </div>

        <Show when={draftDetail()?.draft}>
          {/* Adjust all form */}
          <div class="adjust-all-bar" style={{ padding: '12px 24px', display: 'flex', gap: '16px', "align-items": 'center' }}>
            <label class="adjust-field-label">
              {t('adjustBuyPct')}
              <input
                type="text"
                value={adjustBuyPct()}
                onInput={(e) => setAdjustBuyPct(e.currentTarget.value)}
              />
            </label>
            <label class="adjust-field-label">
              {t('adjustSellPct')}
              <input
                type="text"
                value={adjustSellPct()}
                onInput={(e) => setAdjustSellPct(e.currentTarget.value)}
              />
            </label>
            <button
              type="button"
              class="btn-adjust-all"
              disabled={Boolean(policyError()) || !policyData()?.policy}
              onClick={handleAdjustAll}
            >
              {t('adjustAll')}
            </button>
          </div>

          <div class="table-scroll">
            <table class="rates-table">
              <thead>
                <tr>
                  <th>{t('headerCurrency')}</th>
                  <th class="num">{t('headerBuy')}</th>
                  <th class="num">{t('headerSell')}</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                <For each={draftDetail()?.lines ?? []}>
                  {(line) => (
                    <tr data-draft-line={line.currency}>
                      <td class="code">{line.currency}</td>
                      <td class="num rate-number">
                        {line.buyMode === 'MANUAL' && line.manualBuy
                          ? formatRate(line.manualBuy, getUnitSize(line.currency), line.currency)
                          : line.buyMode}
                      </td>
                      <td class="num rate-number">
                        {line.sellMode === 'MANUAL' && line.manualSell
                          ? formatRate(line.manualSell, getUnitSize(line.currency), line.currency)
                          : line.sellMode}
                      </td>
                      <td>
                        <Show when={isOutOfBound(line.currency)}>
                          <span class="out-of-bound">{t('outOfBound')}</span>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </div>

      <div class="section-note">
        <p>{t('ratesLiveNote')}</p>
      </div>

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
