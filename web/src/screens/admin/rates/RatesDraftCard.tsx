import { For, Show, type JSX } from 'solid-js';
import type * as table from './i18n';
import type { ApiClient } from '../../../shared/api/client';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type {
  RateDraftDetailData,
  RateDraftFilter,
  RateDraftLineDto,
} from '../../../shared/api/generated/rate_draft';
import type { RatePolicyData } from '../../../shared/api/generated/rate_policy';
import type { RatesReferenceData } from '../../../shared/api/generated/rates_reference';
import { wordedError } from '../../../shared/i18n/worded-error';
import { formatRate } from '../../../shared/ui/rate';

export interface OutOfBoundItem {
  currency: string;
  side: 'BUY' | 'SELL';
}

export interface RatesDraftCardProps {
  branch: () => string;
  busy: () => boolean;
  draftDetail: () => RateDraftDetailData | null;
  setDraftDetail: (
    updater: RateDraftDetailData | null | ((prev: RateDraftDetailData | null) => RateDraftDetailData | null),
  ) => void;
  simulateResult: () => {
    id: string;
    isInBound: boolean;
    outOfBound: OutOfBoundItem[];
  } | null;
  setSimulateResult: (
    val: {
      id: string;
      isInBound: boolean;
      outOfBound: OutOfBoundItem[];
    } | null,
  ) => void;
  draftsError: () => string | null;
  policyError: () => string | null;
  policyData: () => RatePolicyData | null;
  pubData: () => PublishedRatesData | null;
  refData: () => RatesReferenceData | null;
  adjustBuyPct: () => string;
  setAdjustBuyPct: (val: string) => void;
  adjustSellPct: () => string;
  setAdjustSellPct: (val: string) => void;
  client: ApiClient;
  writeFor: <B, R>(actionId: string, method: string, path: string) => { send(body: B): Promise<R>; reset(): void };
  readDrafts: (branch: string, gen: number) => Promise<void>;
  readPublished: (branch: string, gen: number) => Promise<void>;
  readGen: () => number;
  guarded: (fn: () => Promise<void>) => () => Promise<void>;
  toast: (msg: string) => void;
  t: (key: keyof typeof table.th) => string;
}

export function RatesDraftCard(props: RatesDraftCardProps): JSX.Element {
  const getUnitSize = (currency: string): string => {
    const pubLine = props.pubData()?.lines?.find((p) => p.currency === currency);
    if (pubLine?.unitSize) return pubLine.unitSize;
    const rRow = props.refData()?.rows?.find((r) => r.code === currency);
    if (rRow?.unit) return String(rRow.unit);
    return '1';
  };

  const isOutOfBound = (currency: string): boolean => {
    const sim = props.simulateResult();
    if (!sim || !props.draftDetail()?.draft || !Array.isArray(sim.outOfBound)) return false;
    return sim.outOfBound.some((item) => item.currency === currency);
  };

  const formatStatus = (status: string | undefined): string => {
    if (!status) return '';
    switch (status) {
      case 'DRAFT':
        return props.t('statusDraft');
      case 'PENDING_APPROVAL':
        return props.t('statusPendingApproval');
      case 'APPROVED':
        return props.t('statusApproved');
      case 'PUBLISHED':
        return props.t('statusPublished');
      case 'REJECTED':
        return props.t('statusRejected');
      default:
        return status;
    }
  };

  const handleNewDraft = props.guarded(async () => {
    const branch = props.branch();
    const gen = props.readGen();
    const curPub = props.pubData();
    const lines = (curPub?.lines ?? []).map((l: PublishedLineDto) => ({
      currency: l.currency,
      buyMode: 'AUTO',
      sellMode: 'AUTO',
      manualBuy: null,
      manualSell: null,
      reason: null,
    }));
    try {
      await props.writeFor('fetchReference', 'POST', '/api/rate-references/fetch').send({ branch });

      const createRes = await props.writeFor<unknown, { id: string; version: number; status: string }>(
        'createDraft',
        'POST',
        '/api/rate-drafts',
      ).send({ branch, lines });

      const detailRes = await props.client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: createRes.id } },
      );
      if (gen !== props.readGen()) return;
      if (detailRes.data) {
        props.setDraftDetail(detailRes.data);
      }

      await props.readDrafts(branch, gen);
      if (gen !== props.readGen()) return;
      props.setSimulateResult(null);
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  const handleSimulate = props.guarded(async () => {
    const draft = props.draftDetail()?.draft;
    if (!draft) return;
    const draftId = draft.id;
    const branch = props.branch();
    const gen = props.readGen();
    try {
      const res = await props.writeFor<
        unknown,
        {
          id: string;
          isInBound: boolean;
          outOfBound: OutOfBoundItem[];
        }
      >('simulateDraft:' + draftId, 'POST', `/api/rate-drafts/${draftId}/simulate`).send({ branch });
      if (gen !== props.readGen() || props.draftDetail()?.draft?.id !== draftId) return;
      props.setSimulateResult(res);
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  const handleSubmit = props.guarded(async () => {
    const draft = props.draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const gen = props.readGen();
    try {
      const res = await props.writeFor<unknown, { requestId?: string; status: string }>(
        'submitDraft:' + draft.id,
        'POST',
        `/api/rate-drafts/${draft.id}/submit`,
      ).send({ branch });
      if (gen !== props.readGen()) return;
      if (res?.status) {
        props.setDraftDetail((prev) =>
          prev && prev.draft ? { ...prev, draft: { ...prev.draft, status: res.status } } : prev,
        );
      }
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  const handlePublish = props.guarded(async () => {
    const draft = props.draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const gen = props.readGen();
    const curVersion = props.pubData()?.publication?.version ?? null;
    try {
      await props.writeFor<unknown, { id: string; version: number }>(
        'publishDraft:' + draft.id,
        'POST',
        `/api/rate-drafts/${draft.id}/publish`,
      ).send({ branch, expectedVersion: curVersion });
      if (gen === props.readGen()) {
        props.setSimulateResult(null);
      }
      await props.readPublished(branch, gen);
      await props.readDrafts(branch, gen);
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  const handleAdjustAll = props.guarded(async () => {
    const draft = props.draftDetail()?.draft;
    const lines = props.draftDetail()?.lines ?? [];
    if (!draft) return;
    const branch = props.branch();
    const gen = props.readGen();
    const policy = props.policyData()?.policy;
    if (!policy) return;
    try {
      await props.writeFor('adjustPolicy', 'POST', '/api/rate-policies').send({
        branch,
        buyAdjustPct: props.adjustBuyPct(),
        sellAdjustPct: props.adjustSellPct(),
        guards: policy.guards,
        maxReferenceAgeHours: policy.maxReferenceAgeHours,
        onStale: policy.onStale,
        staleLimitThb: policy.staleLimitThb,
        reason: policy.reason || props.t('adjustAll'),
      });

      const putLines = lines.map((l: RateDraftLineDto) => ({
        currency: l.currency,
        buyMode: l.buyMode,
        sellMode: l.sellMode,
        manualBuy: l.manualBuy,
        manualSell: l.manualSell,
        reason: l.reason,
      }));

      await props.writeFor('saveDraft:' + draft.id, 'PUT', `/api/rate-drafts/${draft.id}`).send({
        branch,
        expectedVersion: draft.version,
        lines: putLines,
      });
      if (gen === props.readGen()) {
        props.setSimulateResult(null);
      }

      const updated = await props.client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: draft.id } },
      );
      if (gen !== props.readGen()) return;
      if (updated.data) {
        props.setDraftDetail(updated.data);
      }
    } catch (err: unknown) {
      if (gen !== props.readGen()) return;
      props.toast(wordedError(err));
    }
  });

  return (
    <>
      <div class="rates-section">
        <div class="section-header">
          <div class="title-row">
            <div>
              <div class="eyebrow">{props.t('eyebrow')}</div>
              <h2>{props.t('draftTitle')}</h2>
              <Show when={props.draftDetail()?.draft}>
                <div class="live-note">
                  <span class="draft-status-label">
                    {formatStatus(props.draftDetail()?.draft?.status)}
                  </span>
                </div>
              </Show>
            </div>
            <div class="dialog-actions">
              <button
                type="button"
                class="btn-new-draft"
                disabled={props.busy()}
                onClick={handleNewDraft}
              >
                {props.t('newDraft')}
              </button>
              <Show when={props.draftDetail()?.draft}>
                <button
                  type="button"
                  class="btn-simulate"
                  disabled={props.busy()}
                  onClick={handleSimulate}
                >
                  {props.t('simulate')}
                </button>
                <button
                  type="button"
                  class="btn-submit"
                  disabled={props.busy()}
                  onClick={handleSubmit}
                >
                  {props.t('submit')}
                </button>
                <button
                  type="button"
                  class="btn-publish"
                  disabled={props.busy()}
                  onClick={handlePublish}
                >
                  {props.t('publish')}
                </button>
              </Show>
            </div>
          </div>
          <Show when={props.draftDetail()?.draft && props.simulateResult() && props.simulateResult()!.isInBound}>
            <div class="in-bound-status">{props.t('inBound')}</div>
          </Show>
          <Show when={props.draftsError()}>
            <div class="form-error" role="alert">{props.draftsError()}</div>
          </Show>
          <Show when={props.policyError()}>
            <div class="form-error" role="alert">{props.policyError()}</div>
          </Show>
        </div>

        <Show when={props.draftDetail()?.draft}>
          {/* Adjust all form */}
          <div class="adjust-all-bar" style={{ padding: '12px 24px', display: 'flex', gap: '16px', "align-items": 'center' }}>
            <label class="adjust-field-label">
              {props.t('adjustBuyPct')}
              <input
                type="text"
                value={props.adjustBuyPct()}
                onInput={(e) => props.setAdjustBuyPct(e.currentTarget.value)}
              />
            </label>
            <label class="adjust-field-label">
              {props.t('adjustSellPct')}
              <input
                type="text"
                value={props.adjustSellPct()}
                onInput={(e) => props.setAdjustSellPct(e.currentTarget.value)}
              />
            </label>
            <button
              type="button"
              class="btn-adjust-all"
              disabled={props.busy() || Boolean(props.policyError()) || !props.policyData()?.policy}
              onClick={handleAdjustAll}
            >
              {props.t('adjustAll')}
            </button>
          </div>

          <div class="table-scroll">
            <table class="rates-table">
              <thead>
                <tr>
                  <th>{props.t('headerCurrency')}</th>
                  <th class="num">{props.t('headerBuy')}</th>
                  <th class="num">{props.t('headerSell')}</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                <For each={props.draftDetail()?.lines ?? []}>
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
                          <span class="out-of-bound">{props.t('outOfBound')}</span>
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
        <p>{props.t('ratesLiveNote')}</p>
      </div>
    </>
  );
}
