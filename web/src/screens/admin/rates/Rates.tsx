import { createEffect, createSignal, For, onCleanup, onMount, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { PublishedLineDto, PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type {
  RateDraftDetailData,
  RateDraftFilter,
  RateDraftLineDto,
} from '../../../shared/api/generated/rate_draft';
import type { RateDraftsData } from '../../../shared/api/generated/rate_drafts';
import type { RatePolicyData, ValueTierDto } from '../../../shared/api/generated/rate_policy';
import type { RatePolicySaveInput, RatePolicySaveOutput } from '../../../shared/api/generated/rate_policy_save';
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

interface TierFormRow {
  tierNo: 1 | 2 | 3 | 4;
  upperInclusiveThb: string;
  buyMarginPct: string;
  sellMarginPct: string;
}

function emptyValueTiers(): TierFormRow[] {
  return ([1, 2, 3, 4] as const).map((tierNo) => ({
    tierNo,
    upperInclusiveThb: '',
    buyMarginPct: '',
    sellMarginPct: '',
  }));
}

function parsePositiveDecimalParts(raw: string): { intPart: string; fracPart: string } | null {
  const trimmed = raw.trim();
  const m = /^\+?(\d+)(?:\.(\d+))?$/.exec(trimmed);
  if (!m) return null;
  const intPart = m[1]!.replace(/^0+/, '') || '0';
  const fracPart = (m[2] ?? '').replace(/0+$/, '');
  if (intPart === '0' && fracPart === '') return null;
  return { intPart, fracPart };
}

function isDecimalLessThan(
  a: { intPart: string; fracPart: string },
  b: { intPart: string; fracPart: string },
): boolean {
  if (a.intPart.length !== b.intPart.length) {
    return a.intPart.length < b.intPart.length;
  }
  if (a.intPart !== b.intPart) {
    return a.intPart < b.intPart;
  }
  const maxFrac = Math.max(a.fracPart.length, b.fracPart.length);
  const aFrac = a.fracPart.padEnd(maxFrac, '0');
  const bFrac = b.fracPart.padEnd(maxFrac, '0');
  return aFrac < bFrac;
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
    try {
      const res = await client.view<{ permissions?: readonly string[] }>('me');
      if (gen !== readGen) return;
      if (res.data && Array.isArray(res.data.permissions)) {
        setPermissions(res.data.permissions);
      } else {
        setPermissions([]);
      }
    } catch (err: unknown) {
      if (gen !== readGen) return;
      setPermissions([]);
      setPolicyError(wordedError(err));
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
    const gen = readGen;
    const curPub = pubData();
    const lines = (curPub?.lines ?? []).map((l: PublishedLineDto) => ({
      currency: l.currency,
      buyMode: 'AUTO',
      sellMode: 'AUTO',
      manualBuy: null,
      manualSell: null,
      reason: null,
    }));
    try {
      await writeFor('fetchReference', 'POST', '/api/rate-references/fetch').send({ branch });

      const createRes = await writeFor<unknown, { id: string; version: number; status: string }>(
        'createDraft',
        'POST',
        '/api/rate-drafts',
      ).send({ branch, lines });

      const detailRes = await client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: createRes.id } },
      );
      if (gen !== readGen) return;
      if (detailRes.data) {
        setDraftDetail(detailRes.data);
      }

      await readDrafts(branch, gen);
      if (gen !== readGen) return;
      setSimulateResult(null);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const handleSimulate = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const gen = readGen;
    try {
      const res = await writeFor<
        unknown,
        {
          id: string;
          isInBound: boolean;
          outOfBound: OutOfBoundItem[];
        }
      >('simulateDraft:' + draft.id, 'POST', `/api/rate-drafts/${draft.id}/simulate`).send({ branch });
      if (gen !== readGen) return;
      setSimulateResult(res);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const handleSubmit = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const gen = readGen;
    try {
      const res = await writeFor<unknown, { requestId?: string; status: string }>(
        'submitDraft:' + draft.id,
        'POST',
        `/api/rate-drafts/${draft.id}/submit`,
      ).send({ branch });
      if (gen !== readGen) return;
      if (res?.status) {
        setDraftDetail((prev) =>
          prev && prev.draft ? { ...prev, draft: { ...prev.draft, status: res.status } } : prev,
        );
      }
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const handlePublish = async () => {
    const draft = draftDetail()?.draft;
    if (!draft) return;
    const branch = props.branch();
    const gen = readGen;
    const curVersion = pubData()?.publication?.version ?? null;
    try {
      await writeFor<unknown, { id: string; version: number }>(
        'publishDraft:' + draft.id,
        'POST',
        `/api/rate-drafts/${draft.id}/publish`,
      ).send({ branch, expectedVersion: curVersion });
      await readPublished(branch, gen);
      await readDrafts(branch, gen);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const handleAdjustAll = async () => {
    const draft = draftDetail()?.draft;
    const lines = draftDetail()?.lines ?? [];
    if (!draft) return;
    const branch = props.branch();
    const gen = readGen;
    const policy = policyData()?.policy;
    if (!policy) return;
    try {
      await writeFor('adjustPolicy', 'POST', '/api/rate-policies').send({
        branch,
        buyAdjustPct: adjustBuyPct(),
        sellAdjustPct: adjustSellPct(),
        guards: policy.guards,
        maxReferenceAgeHours: policy.maxReferenceAgeHours,
        onStale: policy.onStale,
        staleLimitThb: policy.staleLimitThb,
        reason: policy.reason || t('adjustAll'),
      });

      const putLines = lines.map((l: RateDraftLineDto) => ({
        currency: l.currency,
        buyMode: l.buyMode,
        sellMode: l.sellMode,
        manualBuy: l.manualBuy,
        manualSell: l.manualSell,
        reason: l.reason,
      }));

      await writeFor('saveDraft:' + draft.id, 'PUT', `/api/rate-drafts/${draft.id}`).send({
        branch,
        expectedVersion: draft.version,
        lines: putLines,
      });

      const updated = await client.view<RateDraftDetailData, RateDraftFilter>(
        'rate_draft',
        { branch, filter: { id: draft.id } },
      );
      if (gen !== readGen) return;
      if (updated.data) {
        setDraftDetail(updated.data);
      }
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const nonRisingTierNo = (): 1 | 2 | 3 | null => {
    if (!valueTiersOn()) return null;
    const rows = valueTiers();
    const u1Raw = rows[0]?.upperInclusiveThb ?? '';
    const u2Raw = rows[1]?.upperInclusiveThb ?? '';
    const u3Raw = rows[2]?.upperInclusiveThb ?? '';
    const u1 = parsePositiveDecimalParts(u1Raw);
    const u2 = parsePositiveDecimalParts(u2Raw);
    const u3 = parsePositiveDecimalParts(u3Raw);

    if (u1Raw.trim() !== '' && u1 === null) return 1;
    if (u2Raw.trim() !== '' && u2 === null) return 2;
    if (u1 !== null && u2 !== null && !isDecimalLessThan(u1, u2)) return 2;
    if (u3Raw.trim() !== '' && u3 === null) return 3;
    if (u2 !== null && u3 !== null && !isDecimalLessThan(u2, u3)) return 3;
    if (u1 !== null && u3 !== null && !isDecimalLessThan(u1, u3)) return 3;
    return null;
  };

  const areValueTiersValid = (): boolean => {
    if (!valueTiersOn()) return true;
    const rows = valueTiers();
    if (rows.length !== 4) return false;
    const u1 = parsePositiveDecimalParts(rows[0]!.upperInclusiveThb);
    const u2 = parsePositiveDecimalParts(rows[1]!.upperInclusiveThb);
    const u3 = parsePositiveDecimalParts(rows[2]!.upperInclusiveThb);
    if (!u1 || !u2 || !u3) return false;
    if (!isDecimalLessThan(u1, u2) || !isDecimalLessThan(u2, u3)) return false;
    if (rows[3]!.upperInclusiveThb !== '') return false;
    return rows.every((r) => r.buyMarginPct.trim() !== '' && r.sellMarginPct.trim() !== '');
  };

  const updateTierField = (
    tierNo: 1 | 2 | 3 | 4,
    field: 'upperInclusiveThb' | 'buyMarginPct' | 'sellMarginPct',
    value: string,
  ) => {
    setValueTiers((prev) =>
      prev.map((row) => (row.tierNo === tierNo ? { ...row, [field]: value } : row)),
    );
  };

  const handleSavePolicy = async () => {
    if (!canSavePolicy()) return;
    const policy = policyData()?.policy;
    if (!policy || Boolean(policyError())) return;
    if (!areValueTiersValid()) return;

    const branch = props.branch();
    const gen = readGen;
    const rows = valueTiers();
    const hasAnyTierInput = rows.some(
      (r) => r.upperInclusiveThb !== '' || r.buyMarginPct !== '' || r.sellMarginPct !== '',
    );
    const tiersPayload: ValueTierDto[] =
      !valueTiersOn() && !hasAnyTierInput
        ? []
        : rows.map((r) => ({
            tierNo: r.tierNo,
            upperInclusiveThb:
              r.tierNo === 4 ? null : r.upperInclusiveThb === '' ? null : r.upperInclusiveThb,
            buyMarginPct: r.buyMarginPct,
            sellMarginPct: r.sellMarginPct,
          }));

    const payload: RatePolicySaveInput & { branch: string } = {
      branch,
      buyAdjustPct: adjustBuyPct(),
      sellAdjustPct: adjustSellPct(),
      guards: policy.guards,
      maxReferenceAgeHours: policy.maxReferenceAgeHours,
      onStale: policy.onStale,
      staleLimitThb: policy.staleLimitThb,
      reason: policy.reason || t('savePolicy'),
      valueTiersOn: valueTiersOn(),
      valueTiers: tiersPayload,
      sharpMoveAlertPct: sharpMoveAlertPct() === '' ? null : sharpMoveAlertPct(),
    };

    try {
      await savePolicyWrite.send(payload);
    } catch (err: unknown) {
      if (gen !== readGen) return;
      toast(wordedError(err));
    }
  };

  const openHistory = async () => {
    const branch = props.branch();
    const gen = readGen;
    try {
      const res = await client.view<RatePublicationsData>('rate_publications', { branch });
      if (gen !== readGen) return;
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
      <div class="rates-section">
        <div class="section-header">
          <div class="title-row">
            <div>
              <div class="eyebrow">{t('policyEyebrow')}</div>
              <h2>{t('policyTitle')}</h2>
            </div>
            <div class="dialog-actions">
              <button
                type="button"
                data-action="save-policy"
                data-testid="save-policy"
                disabled={
                  !canSavePolicy() ||
                  Boolean(policyError()) ||
                  !policyData()?.policy ||
                  !areValueTiersValid()
                }
                onClick={handleSavePolicy}
              >
                {t('savePolicy')}
              </button>
            </div>
          </div>
          <Show when={nonRisingTierNo() !== null}>
            <div
              class="form-error"
              role="alert"
              data-field-error={`tier-${nonRisingTierNo()}`}
            >
              {`${t('tierNonRisingPrefix')}${nonRisingTierNo()}${t('tierNonRisingSuffix')}`}
            </div>
          </Show>
        </div>

        <div class="adjust-all-bar" style={{ padding: '12px 24px', display: 'flex', gap: '24px', "align-items": 'center', "flex-wrap": 'wrap' }}>
          <label class="adjust-field-label" style={{ display: 'flex', gap: '8px', "align-items": 'center' }}>
            <input
              type="checkbox"
              name="valueTiersOn"
              data-field="valueTiersOn"
              data-testid="value-tiers-switch"
              checked={valueTiersOn()}
              disabled={!canSavePolicy()}
              onChange={(e) => {
                setValueTiersOn(e.currentTarget.checked);
              }}
            />
            <span>{t('valueTiersSwitchLabel')}</span>
          </label>

          <label class="adjust-field-label">
            {t('sharpMoveThresholdLabel')}
            <input
              type="text"
              name="sharpMoveAlertPct"
              data-field="sharpMoveAlertPct"
              data-testid="sharp-move-threshold"
              value={sharpMoveAlertPct()}
              disabled={!canSavePolicy()}
              onInput={(e) => {
                setSharpMoveAlertPct(e.currentTarget.value);
              }}
            />
          </label>
        </div>

        <div class="table-scroll">
          <table class="rates-table">
            <thead>
              <tr>
                <th>{t('headerTierNo')}</th>
                <th class="num">{t('headerTierUpperThb')}</th>
                <th class="num">{t('headerTierBuyMargin')}</th>
                <th class="num">{t('headerTierSellMargin')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={[1, 2, 3, 4] as const}>
                {(tierNo) => {
                  const row = () => valueTiers()[tierNo - 1]!;
                  return (
                    <tr data-tier-no={tierNo}>
                      <td>{`${t('tierLabelPrefix')}${tierNo}`}</td>
                      <td class="num">
                        <Show
                          when={tierNo < 4}
                          fallback={<span>{t('tierAndAbove')}</span>}
                        >
                          <input
                            type="text"
                            name={`tier-${tierNo}-upperInclusiveThb`}
                            data-field="upperInclusiveThb"
                            data-tier-field="upper"
                            value={row().upperInclusiveThb}
                            disabled={!canSavePolicy()}
                            aria-invalid={nonRisingTierNo() === tierNo ? 'true' : undefined}
                            onInput={(e) =>
                              updateTierField(tierNo, 'upperInclusiveThb', e.currentTarget.value)
                            }
                          />
                          <Show when={nonRisingTierNo() === tierNo}>
                            <div class="field-error" data-error>
                              {`${t('tierNonRisingPrefix')}${tierNo}${t('tierNonRisingSuffix')}`}
                            </div>
                          </Show>
                        </Show>
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name={`tier-${tierNo}-buyMarginPct`}
                          data-field="buyMarginPct"
                          data-tier-field="buy-margin"
                          value={row().buyMarginPct}
                          disabled={!canSavePolicy()}
                          onInput={(e) =>
                            updateTierField(tierNo, 'buyMarginPct', e.currentTarget.value)
                          }
                        />
                      </td>
                      <td class="num">
                        <input
                          type="text"
                          name={`tier-${tierNo}-sellMarginPct`}
                          data-field="sellMarginPct"
                          data-tier-field="sell-margin"
                          value={row().sellMarginPct}
                          disabled={!canSavePolicy()}
                          onInput={(e) =>
                            updateTierField(tierNo, 'sellMarginPct', e.currentTarget.value)
                          }
                        />
                      </td>
                    </tr>
                  );
                }}
              </For>
            </tbody>
          </table>
        </div>
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
