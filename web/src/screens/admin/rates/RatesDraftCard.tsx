import { createEffect, createMemo, createSignal, For, Show, type JSX } from 'solid-js';
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
import { Field } from '../../../shared/ui/Field';
import { formatRate } from '../../../shared/ui/rate';
import './rates-draft.css';

export interface OutOfBoundItem {
  currency: string;
  side: 'BUY' | 'SELL';
}

interface LineFieldErrors {
  manualBuy?: 'manualValueRequired';
  manualSell?: 'manualValueRequired';
  reason?: 'reasonRequired';
}

function isValidManualDecimal(val: string | null | undefined): boolean {
  const trimmed = (val ?? '').trim();
  return /^\d+(\.\d{1,8})?$/.test(trimmed) && /[1-9]/.test(trimmed);
}

function validateLine(line: RateDraftLineDto): LineFieldErrors {
  const errs: LineFieldErrors = {};
  if (line.buyMode === 'MANUAL' && !isValidManualDecimal(line.manualBuy)) {
    errs.manualBuy = 'manualValueRequired';
  }
  if (line.sellMode === 'MANUAL' && !isValidManualDecimal(line.manualSell)) {
    errs.manualSell = 'manualValueRequired';
  }
  if ((line.buyMode === 'MANUAL' || line.sellMode === 'MANUAL') && !(line.reason ?? '').trim()) {
    errs.reason = 'reasonRequired';
  }
  return errs;
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
  const [editLines, setEditLines] = createSignal<RateDraftLineDto[]>([]);
  const [lineErrors, setLineErrors] = createSignal<Record<string, LineFieldErrors>>({});

  createEffect(() => {
    const src = props.draftDetail()?.lines ?? [];
    setEditLines(src.map((l) => ({ ...l })));
    setLineErrors({});
  });

  const isEditable = (): boolean => props.draftDetail()?.draft?.status === 'DRAFT';

  const lineCurrencies = createMemo(
    () => editLines().map((l) => l.currency),
    [] as string[],
    { equals: (a, b) => a.length === b.length && a.every((c, i) => c === b[i]) },
  );

  const updateLine = (currency: string, patch: Partial<RateDraftLineDto>): void => {
    setEditLines((prev) => prev.map((l) => (l.currency === currency ? { ...l, ...patch } : l)));
  };

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

  const handleSaveLines = props.guarded(async () => {
    const draft = props.draftDetail()?.draft;
    if (!draft) return;
    const currentLines = editLines();
    const nextErrors: Record<string, LineFieldErrors> = {};
    let hasError = false;
    for (const l of currentLines) {
      const errs = validateLine(l);
      if (errs.manualBuy || errs.manualSell || errs.reason) {
        nextErrors[l.currency] = errs;
        hasError = true;
      }
    }
    setLineErrors(nextErrors);
    if (hasError) return;

    const branch = props.branch();
    const gen = props.readGen();
    const putLines: RateDraftLineDto[] = currentLines.map((l) => ({
      currency: l.currency,
      buyMode: l.buyMode,
      sellMode: l.sellMode,
      manualBuy: l.buyMode === 'MANUAL' ? l.manualBuy : null,
      manualSell: l.sellMode === 'MANUAL' ? l.manualSell : null,
      reason: l.reason && l.reason.trim() !== '' ? l.reason : null,
    }));

    try {
      const res = await props.writeFor<unknown, { id: string; version: number; status: string }>(
        'saveDraft:' + draft.id,
        'PUT',
        `/api/rate-drafts/${draft.id}`,
      ).send({
        branch,
        expectedVersion: draft.version,
        lines: putLines,
      });
      if (gen !== props.readGen()) return;
      props.setSimulateResult(null);
      props.setDraftDetail((prev) =>
        prev && prev.draft
          ? {
              ...prev,
              draft: { ...prev.draft, version: res?.version ?? prev.draft.version },
              lines: putLines,
            }
          : prev,
      );
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
                <Show when={isEditable()}>
                  <button
                    type="button"
                    class="btn-save-lines"
                    disabled={props.busy()}
                    onClick={handleSaveLines}
                  >
                    {props.t('saveLines')}
                  </button>
                </Show>
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
          <div class="adjust-all-bar adjust-all-bar--draft">
            <Field label={props.t('adjustBuyPct')} class="adjust-field-label">
              <input
                type="text"
                value={props.adjustBuyPct()}
                onInput={(e) => props.setAdjustBuyPct(e.currentTarget.value)}
              />
            </Field>
            <Field label={props.t('adjustSellPct')} class="adjust-field-label">
              <input
                type="text"
                value={props.adjustSellPct()}
                onInput={(e) => props.setAdjustSellPct(e.currentTarget.value)}
              />
            </Field>
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
                  <th>{props.t('reasonLabel')}</th>
                </tr>
              </thead>
              <tbody>
                <For each={lineCurrencies()}>
                  {(currency) => {
                    const line = () => editLines().find((l) => l.currency === currency);
                    const errs = () => lineErrors()[currency];
                    return (
                      <tr data-draft-line={currency}>
                        <td class="code">{currency}</td>
                        <td class="num rate-number">
                          <Show
                            when={isEditable()}
                            fallback={
                              line()?.buyMode === 'MANUAL' && line()?.manualBuy
                                ? formatRate(line()!.manualBuy!, getUnitSize(currency), currency)
                                : line()?.buyMode === 'MANUAL'
                                  ? props.t('modeManual')
                                  : props.t('modeAuto')
                            }
                          >
                            <select
                              name="buyMode"
                              value={line()?.buyMode ?? 'AUTO'}
                              onChange={(e) =>
                                updateLine(currency, {
                                  buyMode: e.currentTarget.value,
                                  manualBuy: e.currentTarget.value === 'MANUAL' ? (line()?.manualBuy ?? '') : null,
                                })
                              }
                            >
                              <option value="AUTO">{props.t('modeAuto')}</option>
                              <option value="MANUAL">{props.t('modeManual')}</option>
                            </select>
                            <Show when={line()?.buyMode === 'MANUAL'}>
                              <input
                                type="text"
                                name="manualBuy"
                                value={line()?.manualBuy ?? ''}
                                onInput={(e) => updateLine(currency, { manualBuy: e.currentTarget.value })}
                              />
                              <Show when={isValidManualDecimal(line()?.manualBuy)}>
                                <span>{formatRate(line()!.manualBuy!, getUnitSize(currency), currency)}</span>
                              </Show>
                            </Show>
                            <Show when={errs()?.manualBuy}>
                              <span class="field-error" data-field-error="manualBuy">
                                {props.t(errs()!.manualBuy!)}
                              </span>
                            </Show>
                          </Show>
                        </td>
                        <td class="num rate-number">
                          <Show
                            when={isEditable()}
                            fallback={
                              line()?.sellMode === 'MANUAL' && line()?.manualSell
                                ? formatRate(line()!.manualSell!, getUnitSize(currency), currency)
                                : line()?.sellMode === 'MANUAL'
                                  ? props.t('modeManual')
                                  : props.t('modeAuto')
                            }
                          >
                            <select
                              name="sellMode"
                              value={line()?.sellMode ?? 'AUTO'}
                              onChange={(e) =>
                                updateLine(currency, {
                                  sellMode: e.currentTarget.value,
                                  manualSell: e.currentTarget.value === 'MANUAL' ? (line()?.manualSell ?? '') : null,
                                })
                              }
                            >
                              <option value="AUTO">{props.t('modeAuto')}</option>
                              <option value="MANUAL">{props.t('modeManual')}</option>
                            </select>
                            <Show when={line()?.sellMode === 'MANUAL'}>
                              <input
                                type="text"
                                name="manualSell"
                                value={line()?.manualSell ?? ''}
                                onInput={(e) => updateLine(currency, { manualSell: e.currentTarget.value })}
                              />
                              <Show when={isValidManualDecimal(line()?.manualSell)}>
                                <span>{formatRate(line()!.manualSell!, getUnitSize(currency), currency)}</span>
                              </Show>
                            </Show>
                            <Show when={errs()?.manualSell}>
                              <span class="field-error" data-field-error="manualSell">
                                {props.t(errs()!.manualSell!)}
                              </span>
                            </Show>
                          </Show>
                        </td>
                        <td>
                          <Show when={isEditable()} fallback={line()?.reason ?? ''}>
                            <input
                              type="text"
                              name="reason"
                              value={line()?.reason ?? ''}
                              onInput={(e) => updateLine(currency, { reason: e.currentTarget.value })}
                            />
                            <Show when={errs()?.reason}>
                              <span class="field-error" data-field-error="reason">
                                {props.t(errs()!.reason!)}
                              </span>
                            </Show>
                          </Show>
                          <Show when={isOutOfBound(currency)}>
                            <span class="out-of-bound">{props.t('outOfBound')}</span>
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
      </div>

      <div class="section-note">
        <p>{props.t('ratesLiveNote')}</p>
      </div>
    </>
  );
}
