// MK-8a (#1645): Admin Marketing Campaigns screen M01 list + M02 campaign page
// (design `docs/design/blueprint/marketing-campaigns.md` §4.5, §9.1 MK-8; D-4, D-9, D-15).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  CampaignData,
  CampaignDetail,
  CampaignFilter,
} from '../../../shared/api/generated/campaign';
import type {
  CampaignCreateInput,
  CampaignCreateOutput,
} from '../../../shared/api/generated/campaign_create';
import type {
  CampaignPauseInput,
  CampaignPauseOutput,
} from '../../../shared/api/generated/campaign_pause';
import type {
  CampaignResumeInput,
  CampaignResumeOutput,
} from '../../../shared/api/generated/campaign_resume';
import type {
  CampaignSimulateInput,
  CampaignSimulateOutput,
} from '../../../shared/api/generated/campaign_simulate';
import type {
  CampaignSubmitInput,
  CampaignSubmitOutput,
} from '../../../shared/api/generated/campaign_submit';
import type {
  CampaignVersionNewInput,
  CampaignVersionNewOutput,
} from '../../../shared/api/generated/campaign_version_new';
import type {
  CampaignVersionSaveInput,
  CampaignVersionSaveOutput,
} from '../../../shared/api/generated/campaign_version_save';
import type {
  CampaignListItem,
  CampaignsData,
  CampaignsFilter,
} from '../../../shared/api/generated/campaigns';
import type {
  CouponIssueOutput,
  IssuedCouponDto,
} from '../../../shared/api/generated/coupon_issue';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import type { AdminMenuProps } from '../menus';
import * as table from './i18n';
import './campaigns.css';

const t = screenT('admin.campaigns', table);

const ERROR_KEYS: Record<string, string> = {
  simulationRequired: 'error.simulationRequired',
  staleVersion: 'error.staleVersion',
  approvalRequired: 'error.approvalRequired',
  validationFailed: 'error.validationFailed',
  outOfScope: 'error.outOfScope',
  idempotencyConflict: 'error.idempotencyConflict',
  couponInvalid: 'error.couponInvalid',
};

function campaignError(err: unknown): string {
  if (err instanceof ApiError) {
    if (isServerFailure(err)) {
      return wordedError(err);
    }
    const mapped = ERROR_KEYS[err.code];
    if (mapped) {
      const text = t(mapped as Parameters<typeof t>[0]);
      if (text && text !== mapped) return text;
    }
  }
  return wordedError(err);
}

function formatMoney(value: string | null | undefined): string {
  if (value === null || value === undefined || value === '') return '—';
  const isDecimal = /^-?\d+(\.\d+)?$/.test(value);
  if (!isDecimal) return value;
  const [intPart, decPart] = value.split('.');
  const isNegative = intPart.startsWith('-');
  const unsignedInt = isNegative ? intPart.slice(1) : intPart;
  const groupedInt = unsignedInt.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${isNegative ? '-' : ''}${groupedInt}${decPart !== undefined ? `.${decPart}` : ''}`;
}

function statusLabel(status: string): string {
  const key = `status.${status.toLowerCase()}` as Parameters<typeof t>[0];
  const text = t(key);
  if (text && text !== key) return text;
  return status;
}

function toBangkokStart(dateStr: string): string {
  if (!dateStr) return '';
  if (dateStr.includes('T')) return dateStr;
  return `${dateStr}T00:00:00+07:00`;
}

function toBangkokEnd(dateStr: string): string {
  if (!dateStr) return '';
  if (dateStr.includes('T')) return dateStr;
  return `${dateStr}T23:59:59+07:00`;
}

export function Campaigns(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  // M01 List state
  const [campaignsList, setCampaignsList] = createSignal<CampaignListItem[]>([]);
  const [loadingList, setLoadingList] = createSignal(true);
  const [listError, setListError] = createSignal<string | null>(null);

  // Create Campaign state
  const [isCreating, setIsCreating] = createSignal(false);
  const [createName, setCreateName] = createSignal('');
  const [createBudgetThb, setCreateBudgetThb] = createSignal('');
  const [createStartsAt, setCreateStartsAt] = createSignal('');
  const [createEndsAt, setCreateEndsAt] = createSignal('');
  const [createStackGroup, setCreateStackGroup] = createSignal('DEFAULT');
  const [createPriority, setCreatePriority] = createSignal<number>(10);
  const [createBranches, setCreateBranches] = createSignal<string[]>(['MAIN']);
  const [createError, setCreateError] = createSignal<string | null>(null);

  // M02 Detail state
  const [selectedCampaignId, setSelectedCampaignId] = createSignal<string | null>(null);
  const [campaignDetail, setCampaignDetail] = createSignal<CampaignDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = createSignal(false);
  const [detailError, setDetailError] = createSignal<string | null>(null);

  // Detail Form inputs
  const [formName, setFormName] = createSignal('');
  const [formBudgetThb, setFormBudgetThb] = createSignal('');
  const [formStartsAt, setFormStartsAt] = createSignal('');
  const [formEndsAt, setFormEndsAt] = createSignal('');
  const [formStackGroup, setFormStackGroup] = createSignal('DEFAULT');
  const [formPriority, setFormPriority] = createSignal(0);
  const [formBranches, setFormBranches] = createSignal<string[]>([]);
  const [isEditedAfterSimulation, setIsEditedAfterSimulation] = createSignal(false);

  // Coupon issue inputs
  const [couponRuleId, setCouponRuleId] = createSignal('');
  const [couponCount, setCouponCount] = createSignal(1);
  const [couponExpiresAt, setCouponExpiresAt] = createSignal('');

  // Issued coupons from couponIssue
  const [issuedCoupons, setIssuedCoupons] = createSignal<IssuedCouponDto[] | null>(null);

  // Retain Idempotency-Key across retries
  let pendingKey: { action: string; bodyString: string; key: string } | null = null;

  function getIdempotencyKey(action: string, body: unknown): string {
    const bodyString = JSON.stringify(body ?? {});
    if (!pendingKey || pendingKey.action !== action || pendingKey.bodyString !== bodyString) {
      pendingKey = {
        action,
        bodyString,
        key: crypto.randomUUID(),
      };
    }
    return pendingKey.key;
  }

  function clearPendingKey(): void {
    pendingKey = null;
  }

  let readingList = 0;
  async function loadList(branch: string | undefined, from: string, to: string) {
    const mine = ++readingList;
    setLoadingList(true);
    setListError(null);
    try {
      const res = await api.view<CampaignsData, CampaignsFilter>('campaigns', {
        branch,
        filter: { from, to },
      });
      if (mine !== readingList) return;
      const list = Array.isArray(res.data?.campaigns) ? res.data.campaigns : [];
      setCampaignsList(list);
    } catch (err: unknown) {
      if (mine !== readingList) return;
      setCampaignsList([]);
      setListError(campaignError(err));
    } finally {
      if (mine === readingList) {
        setLoadingList(false);
      }
    }
  }

  let readingDetail = 0;
  async function loadCampaignDetail(id: string, keepUnsaved = false) {
    const mine = ++readingDetail;
    setLoadingDetail(true);
    setDetailError(null);
    try {
      const res = await api.view<CampaignData, CampaignFilter>('campaign', {
        branch: props.branch?.(),
        filter: { id },
      });
      if (mine !== readingDetail) return;
      const detail = res.data?.campaign ?? null;
      setCampaignDetail(detail);

      if (detail) {
        const v = detail.versions[0];
        if (!keepUnsaved) {
          setFormName(detail.name ?? '');
          setFormBudgetThb(v?.budgetThb ?? '');
          setFormStartsAt(v?.startsAt ?? '');
          setFormEndsAt(v?.endsAt ?? '');
          setFormStackGroup(v?.stackGroup ?? 'DEFAULT');
          setFormPriority(v?.priority ?? 0);
          setFormBranches(v?.branches ? [...v.branches] : []);
          setIsEditedAfterSimulation(false);
        }
        if (v?.rules && v.rules.length > 0) {
          setCouponRuleId(v.rules[0].id);
        } else {
          setCouponRuleId('');
        }
        if (v?.endsAt) {
          setCouponExpiresAt(v.endsAt.includes('T') ? v.endsAt.slice(0, 10) : v.endsAt);
        }
      }
    } catch (err: unknown) {
      if (mine !== readingDetail) return;
      setDetailError(campaignError(err));
    } finally {
      if (mine === readingDetail) {
        setLoadingDetail(false);
      }
    }
  }

  let lastBranch: string | undefined = undefined;
  createEffect(() => {
    if (props.shown && !props.shown()) return;
    const branch = props.branch?.();
    const p = props.period();
    if (lastBranch !== undefined && branch !== lastBranch) {
      setSelectedCampaignId(null);
      setCampaignDetail(null);
      setCampaignsList([]);
    }
    lastBranch = branch;
    void loadList(branch, p.from, p.to);
  });

  const currentVersion = () => campaignDetail()?.versions[0] ?? null;

  const currentStatus = () => {
    if (loadingDetail()) return 'LOADING';
    const v = currentVersion();
    return v?.status ?? 'UNKNOWN';
  };

  const isSubmitDisabled = () => {
    return currentStatus() !== 'SIMULATED' || isEditedAfterSimulation();
  };

  function startCreate() {
    setIsCreating(true);
    setSelectedCampaignId(null);
    setCampaignDetail(null);
    setIssuedCoupons(null);
    setCreateName('');
    setCreateBudgetThb('');
    setCreateStartsAt('');
    setCreateEndsAt('');
    setCreateStackGroup('DEFAULT');
    setCreatePriority(10);
    setCreateBranches(['MAIN']);
    setCreateError(null);
    clearPendingKey();
  }

  function openCampaign(campaignId: string) {
    setIsCreating(false);
    setSelectedCampaignId(campaignId);
    setIssuedCoupons(null);
    clearPendingKey();
    void loadCampaignDetail(campaignId, false);
  }

  function handleInputEdit(setter: (val: string) => void, val: string) {
    setter(val);
    setIsEditedAfterSimulation(true);
  }

  async function handleCreate() {
    const rawBranches = createBranches() as unknown;
    const branches: string[] = Array.isArray(rawBranches)
      ? rawBranches.map(String)
      : typeof rawBranches === 'string' && rawBranches.length > 0
        ? [rawBranches]
        : [];
    const body: CampaignCreateInput = {
      name: createName(),
      startsAt: toBangkokStart(createStartsAt()),
      endsAt: toBangkokEnd(createEndsAt()),
      budgetThb: createBudgetThb(),
      stackGroup: createStackGroup() || 'DEFAULT',
      priority: Number(createPriority()) || 0,
      branches,
    };
    const key = getIdempotencyKey('create', body);
    setCreateError(null);
    try {
      const res = await api.command<CampaignCreateInput, CampaignCreateOutput>(
        'POST',
        '/api/campaigns',
        body,
        key,
      );
      clearPendingKey();
      setIsCreating(false);
      if (res?.id) {
        openCampaign(res.id);
      } else {
        void loadList(props.branch?.(), props.period().from, props.period().to);
      }
    } catch (err: unknown) {
      setCreateError(campaignError(err));
    }
  }

  async function handleSave() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const rawBranches = formBranches() as unknown;
    const branches: string[] = Array.isArray(rawBranches)
      ? rawBranches.map(String)
      : typeof rawBranches === 'string' && rawBranches.length > 0
        ? [rawBranches]
        : [];
    const body: CampaignVersionSaveInput = {
      campaignId,
      startsAt: toBangkokStart(formStartsAt()),
      endsAt: toBangkokEnd(formEndsAt()),
      budgetThb: formBudgetThb(),
      stackGroup: formStackGroup(),
      priority: Number(formPriority()) || 0,
      branches,
    };
    const key = getIdempotencyKey('save', body);
    setDetailError(null);
    try {
      await api.command<CampaignVersionSaveInput, CampaignVersionSaveOutput>(
        'PUT',
        `/api/campaigns/${campaignId}/draft`,
        body,
        key,
      );
      clearPendingKey();
      setIsEditedAfterSimulation(false);
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadCampaignDetail(campaignId, true);
      }
      setDetailError(campaignError(err));
    }
  }

  async function handleSimulate() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignSimulateInput = {
      campaignId,
      scenarios: [],
    };
    const key = getIdempotencyKey('simulate', body);
    setDetailError(null);
    try {
      await api.command<CampaignSimulateInput, CampaignSimulateOutput>(
        'POST',
        `/api/campaigns/${campaignId}/simulate`,
        body,
        key,
      );
      clearPendingKey();
      setIsEditedAfterSimulation(false);
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  async function handleSubmit() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignSubmitInput = {
      campaignId,
    };
    const key = getIdempotencyKey('submit', body);
    setDetailError(null);
    try {
      await api.command<CampaignSubmitInput, CampaignSubmitOutput>(
        'POST',
        `/api/campaigns/${campaignId}/submit`,
        body,
        key,
      );
      clearPendingKey();
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  async function handlePause() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignPauseInput = {
      campaignId,
    };
    const key = getIdempotencyKey('pause', body);
    setDetailError(null);
    try {
      await api.command<CampaignPauseInput, CampaignPauseOutput>(
        'POST',
        `/api/campaigns/${campaignId}/pause`,
        body,
        key,
      );
      clearPendingKey();
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  async function handleResume() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignResumeInput = {
      campaignId,
    };
    const key = getIdempotencyKey('resume', body);
    setDetailError(null);
    try {
      await api.command<CampaignResumeInput, CampaignResumeOutput>(
        'POST',
        `/api/campaigns/${campaignId}/resume`,
        body,
        key,
      );
      clearPendingKey();
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  async function handleNewVersion() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignVersionNewInput = {
      campaignId,
    };
    const key = getIdempotencyKey('newVersion', body);
    setDetailError(null);
    try {
      await api.command<CampaignVersionNewInput, CampaignVersionNewOutput>(
        'POST',
        `/api/campaigns/${campaignId}/versions`,
        body,
        key,
      );
      clearPendingKey();
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  async function handleIssueCoupons() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const ruleId = couponRuleId() || currentVersion()?.rules?.[0]?.id || '';
    const count = Number(couponCount()) || 1;
    const expiresAt = toBangkokEnd(couponExpiresAt());
    const body = {
      ruleId,
      count,
      expiresAt,
    };
    const key = getIdempotencyKey('issue', body);
    setDetailError(null);
    try {
      const res = await api.command<typeof body, CouponIssueOutput>(
        'POST',
        `/api/campaigns/${campaignId}/coupons`,
        body,
        key,
      );
      clearPendingKey();
      setIssuedCoupons(res?.coupons ?? []);
    } catch (err: unknown) {
      setDetailError(campaignError(err));
    }
  }

  return (
    <div class="xc-campaigns-screen">
      <Show when={selectedCampaignId()} fallback={
        <Show when={isCreating()} fallback={
          <>
            {/* M01 List View */}
            <div class="xc-campaigns-header">
              <h3>{t('title')}</h3>
              <button
                type="button"
                class="xc-btn-primary"
                onClick={startCreate}
              >
                {t('create')}
              </button>
            </div>

            <Show when={listError()}>
              {(errMessage) => (
                <div class="xc-campaigns-error">
                  <span>{errMessage()}</span>
                  <button
                    type="button"
                    class="xc-campaigns-retry-btn"
                    onClick={() => void loadList(props.branch?.(), props.period().from, props.period().to)}
                  >
                    {t('retry')}
                  </button>
                </div>
              )}
            </Show>

            <Show when={loadingList() && campaignsList().length === 0}>
              <div class="xc-campaigns-loading">{t('loading')}</div>
            </Show>

            <Show when={!loadingList() && !listError() && campaignsList().length === 0}>
              <div class="xc-campaigns-empty">
                <span>{t('empty')}</span>
                <button
                  type="button"
                  class="xc-btn-primary"
                  onClick={startCreate}
                >
                  {t('create')}
                </button>
              </div>
            </Show>

            <Show when={campaignsList().length > 0}>
              <div class="xc-campaigns-table-container">
                <table class="xc-campaigns-table">
                  <thead>
                    <tr>
                      <th>{t('headerName')}</th>
                      <th class="text-right">{t('headerVersion')}</th>
                      <th>{t('headerStatus')}</th>
                      <th class="text-right">{t('headerBudget')}</th>
                      <th class="text-right">{t('headerUsed')}</th>
                      <th class="text-right">{t('headerLeft')}</th>
                      <th class="text-right">{t('headerUses')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={campaignsList()}>
                      {(row) => (
                        <tr
                          data-campaign-id={row.id}
                          class="xc-campaign-row"
                          onClick={() => openCampaign(row.id)}
                        >
                          <td>{row.name}</td>
                          <td class="text-right">{row.version}</td>
                          <td>
                            <span class={`xc-status-badge xc-status-${row.status.toLowerCase()}`}>
                              {statusLabel(row.status)}
                            </span>
                          </td>
                          <td class="text-right">{formatMoney(row.budgetThb)}</td>
                          <td class="text-right">{formatMoney(row.budgetUsedThb)}</td>
                          <td class="text-right">{formatMoney(row.budgetLeftThb)}</td>
                          <td class="text-right">
                            {row.uses}
                            <Show when={row.maxUses !== null && row.maxUses !== undefined}>
                              {' / '}
                              {row.maxUses}
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </>
        }>
          {/* Create Campaign View */}
          <div class="xc-campaign-create">
            <div class="xc-detail-header">
              <h3>{t('createTitle')}</h3>
              <div class="xc-detail-actions">
                <button
                  type="button"
                  class="xc-btn-secondary"
                  onClick={() => setIsCreating(false)}
                >
                  {t('back')}
                </button>
                <button
                  type="button"
                  class="xc-btn-primary"
                  onClick={handleCreate}
                >
                  {t('create')}
                </button>
              </div>
            </div>

            <Show when={createError()}>
              {(errMessage) => (
                <div class="xc-campaigns-error">
                  <span>{errMessage()}</span>
                </div>
              )}
            </Show>

            <div class="xc-form-grid">
              <div class="xc-form-group">
                <label>{t('fieldName')}</label>
                <input
                  type="text"
                  name="name"
                  data-field="name"
                  class="xc-input"
                  value={createName()}
                  onInput={(e) => setCreateName(e.currentTarget.value)}
                  onChange={(e) => setCreateName(e.currentTarget.value)}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldStartsAt')}</label>
                <input
                  type="date"
                  name="startsAt"
                  data-field="startsAt"
                  class="xc-input"
                  value={createStartsAt()}
                  onInput={(e) => setCreateStartsAt(e.currentTarget.value)}
                  onChange={(e) => setCreateStartsAt(e.currentTarget.value)}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldEndsAt')}</label>
                <input
                  type="date"
                  name="endsAt"
                  data-field="endsAt"
                  class="xc-input"
                  value={createEndsAt()}
                  onInput={(e) => setCreateEndsAt(e.currentTarget.value)}
                  onChange={(e) => setCreateEndsAt(e.currentTarget.value)}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldBudgetThb')}</label>
                <input
                  type="text"
                  name="budgetThb"
                  data-field="budgetThb"
                  class="xc-input"
                  value={createBudgetThb()}
                  onInput={(e) => setCreateBudgetThb(e.currentTarget.value)}
                  onChange={(e) => setCreateBudgetThb(e.currentTarget.value)}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldStackGroup')}</label>
                <input
                  type="text"
                  name="stackGroup"
                  data-field="stackGroup"
                  class="xc-input"
                  value={createStackGroup()}
                  onInput={(e) => setCreateStackGroup(e.currentTarget.value)}
                  onChange={(e) => setCreateStackGroup(e.currentTarget.value)}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldPriority')}</label>
                <input
                  type="number"
                  name="priority"
                  data-field="priority"
                  class="xc-input"
                  value={createPriority()}
                  onInput={(e) => setCreatePriority(Number(e.currentTarget.value))}
                  onChange={(e) => setCreatePriority(Number(e.currentTarget.value))}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('fieldBranches')}</label>
                <input
                  type="text"
                  name="branches"
                  data-field="branches"
                  class="xc-input"
                  value={createBranches().join(', ')}
                  onInput={(e) => {
                    const raw = e.currentTarget.value;
                    const parts = raw.split(',').map((s) => s.trim()).filter(Boolean);
                    setCreateBranches(parts.length > 0 ? parts : (raw ? [raw] : []));
                  }}
                  onChange={(e) => {
                    const raw = e.currentTarget.value;
                    const parts = raw.split(',').map((s) => s.trim()).filter(Boolean);
                    setCreateBranches(parts.length > 0 ? parts : (raw ? [raw] : []));
                  }}
                />
              </div>
            </div>
          </div>
        </Show>
      }>
        {/* M02 Detail View */}
        <div class="xc-campaign-detail">
          <div class="xc-detail-header">
            <div>
              <button
                type="button"
                class="xc-btn-secondary"
                onClick={() => {
                  setSelectedCampaignId(null);
                  setCampaignDetail(null);
                  setIssuedCoupons(null);
                  void loadList(props.branch?.(), props.period().from, props.period().to);
                }}
              >
                {t('back')}
              </button>
              <span
                class={`xc-status-badge xc-status-${currentStatus().toLowerCase()}`}
                data-testid="campaign-status"
              >
                {statusLabel(currentStatus())}
              </span>
            </div>

            <div class="xc-detail-actions">
              <button
                type="button"
                class="xc-btn-primary"
                onClick={handleSave}
              >
                {t('save')}
              </button>

              <button
                type="button"
                class="xc-btn-secondary"
                onClick={handleSimulate}
              >
                {t('simulate')}
              </button>

              <button
                type="button"
                class="xc-btn-success"
                disabled={isSubmitDisabled()}
                onClick={handleSubmit}
              >
                {t('submit')}
              </button>

              <Show when={currentStatus() === 'ACTIVE'}>
                <button
                  type="button"
                  class="xc-btn-warning"
                  onClick={handlePause}
                >
                  {t('pause')}
                </button>
              </Show>

              <Show when={currentStatus() === 'PAUSED'}>
                <button
                  type="button"
                  class="xc-btn-success"
                  onClick={handleResume}
                >
                  {t('resume')}
                </button>
              </Show>

              <Show when={currentStatus() === 'ACTIVE' || currentStatus() === 'PAUSED'}>
                <button
                  type="button"
                  class="xc-btn-secondary"
                  onClick={handleNewVersion}
                >
                  {t('newVersion')}
                </button>
              </Show>
            </div>
          </div>

          <Show when={detailError()}>
            {(errMessage) => (
              <div class="xc-campaigns-error">
                <span>{errMessage()}</span>
              </div>
            )}
          </Show>

          <Show when={loadingDetail()}>
            <div class="xc-campaigns-loading">{t('loading')}</div>
          </Show>

          <div class="xc-form-grid">
            <div class="xc-form-group">
              <label>{t('fieldName')}</label>
              <input
                type="text"
                name="name"
                data-field="name"
                class="xc-input"
                value={formName()}
                onInput={(e) => handleInputEdit(setFormName, e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label>{t('fieldBudgetThb')}</label>
              <input
                type="text"
                name="budgetThb"
                data-field="budgetThb"
                class="xc-input"
                value={formBudgetThb()}
                onInput={(e) => handleInputEdit(setFormBudgetThb, e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label>{t('fieldStartsAt')}</label>
              <input
                type="text"
                name="startsAt"
                data-field="startsAt"
                class="xc-input"
                value={formStartsAt()}
                onInput={(e) => handleInputEdit(setFormStartsAt, e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label>{t('fieldEndsAt')}</label>
              <input
                type="text"
                name="endsAt"
                data-field="endsAt"
                class="xc-input"
                value={formEndsAt()}
                onInput={(e) => handleInputEdit(setFormEndsAt, e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label>{t('fieldStackGroup')}</label>
              <input
                type="text"
                name="stackGroup"
                data-field="stackGroup"
                class="xc-input"
                value={formStackGroup()}
                onInput={(e) => handleInputEdit(setFormStackGroup, e.currentTarget.value)}
              />
            </div>

            <div class="xc-form-group">
              <label>{t('fieldPriority')}</label>
              <input
                type="number"
                name="priority"
                data-field="priority"
                class="xc-input"
                value={formPriority()}
                onInput={(e) => {
                  setFormPriority(Number(e.currentTarget.value));
                  setIsEditedAfterSimulation(true);
                }}
              />
            </div>
          </div>

          {/* Coupon Issue section */}
          <div class="xc-coupon-issue-section">
            <h4>{t('issueCoupons')}</h4>
            <div class="xc-form-grid">
              <div class="xc-form-group">
                <label>{t('couponRule')}</label>
                <select
                  name="ruleId"
                  class="xc-select"
                  value={couponRuleId()}
                  onInput={(e) => setCouponRuleId(e.currentTarget.value)}
                  onChange={(e) => setCouponRuleId(e.currentTarget.value)}
                >
                  <For each={currentVersion()?.rules ?? []}>
                    {(r) => (
                      <option value={r.id}>
                        {r.benefitType} (#{r.ruleNo})
                      </option>
                    )}
                  </For>
                </select>
              </div>

              <div class="xc-form-group">
                <label>{t('couponCount')}</label>
                <input
                  type="number"
                  name="count"
                  class="xc-input"
                  min="1"
                  value={couponCount()}
                  onInput={(e) => setCouponCount(Number(e.currentTarget.value))}
                  onChange={(e) => setCouponCount(Number(e.currentTarget.value))}
                />
              </div>

              <div class="xc-form-group">
                <label>{t('couponExpiresAt')}</label>
                <input
                  type="date"
                  name="expiresAt"
                  class="xc-input"
                  value={couponExpiresAt()}
                  onInput={(e) => setCouponExpiresAt(e.currentTarget.value)}
                  onChange={(e) => setCouponExpiresAt(e.currentTarget.value)}
                />
              </div>
            </div>

            <button
              type="button"
              class="xc-btn-secondary"
              onClick={handleIssueCoupons}
            >
              {t('issueCoupons')}
            </button>
          </div>

          {/* Issued coupons list */}
          <Show when={issuedCoupons()}>
            {(coupons) => (
              <div class="xc-issued-coupons">
                <h4>{t('issuedCouponsTitle')}</h4>
                <For each={coupons()}>
                  {(c) => (
                    <div class="xc-coupon-item">
                      <span class="xc-coupon-id">{c.id}</span>
                      <Show when={c.code !== undefined && c.code !== null && c.code !== ''}>
                        <span class="xc-coupon-code">{c.code}</span>
                        <button
                          type="button"
                          class="xc-btn-secondary"
                          onClick={() => navigator.clipboard?.writeText?.(c.code ?? '')}
                        >
                          {t('copy')}
                        </button>
                      </Show>
                    </div>
                  )}
                </For>
              </div>
            )}
          </Show>
        </div>
      </Show>
    </div>
  );
}
