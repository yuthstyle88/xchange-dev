// MK-8a (#1645): Admin Marketing Campaigns screen M01 list + M02 campaign page
// (design `docs/design/blueprint/marketing-campaigns.md` §4.5, §9.1 MK-8; D-4, D-9, D-15).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
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
import { fieldError, type FieldMessage } from '../../../shared/i18n/field-error';
import { isServerFailure } from '../../../shared/i18n/worded-error';
import { checkDecimal, numeric } from '../../../shared/ui/bounded';
import type { AdminMenuProps } from '../menus';
import { CampaignRulesEditor } from './CampaignRulesEditor';
import {
  isKnownBenefitType,
  ruleErrors,
  ruleFormOf,
  ruleInputOf,
  type RuleError,
  type RuleForm,
} from './campaign-rules';
import * as table from './i18n';
import './campaigns.css';

const t = screenT('admin.campaigns', table);

const BUDGET_BOUND = numeric(20, 2, 'nonNegative');

const ERROR_KEYS: Record<string, string> = {
  simulationRequired: 'error.simulationRequired',
  staleVersion: 'error.staleVersion',
  approvalRequired: 'error.approvalRequired',
  validationFailed: 'error.validationFailed',
  outOfScope: 'error.outOfScope',
  idempotencyConflict: 'error.idempotencyConflict',
  couponInvalid: 'error.couponInvalid',
};

function campaignFieldError(err: unknown): FieldMessage {
  const labels = {
    name: t('fieldName'),
    startsAt: t('fieldStartsAt'),
    endsAt: t('fieldEndsAt'),
    budgetThb: t('fieldBudgetThb'),
    priority: t('fieldPriority'),
    branches: t('fieldBranches'),
    rules: t('rulesTitle'),
    count: t('couponCount'),
    expiresAt: t('couponExpiresAt'),
    amount: t('simAmount'),
  };
  if (err instanceof ApiError && !isServerFailure(err)) {
    if (err.code === 'validationFailed' && err.field === 'rules') {
      const msg = err.message.trim();
      return {
        field: 'rules',
        message: msg !== '' ? msg : fieldError(err, labels).message,
      };
    }
    if (err.code !== 'validationFailed') {
      const mapped = ERROR_KEYS[err.code];
      if (mapped) {
        const text = t(mapped as Parameters<typeof t>[0]);
        if (text && text !== mapped) return { message: text };
      }
    }
  }
  return fieldError(err, labels);
}

function campaignError(err: unknown): string {
  return campaignFieldError(err).message;
}

function localFieldError(field: string): string {
  return campaignFieldError(
    new ApiError({ status: 400, code: 'validationFailed', field, message: '' }),
  ).message;
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

const BANGKOK_PARTS = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Bangkok',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hourCycle: 'h23',
});

function toBangkokWallClock(isoStr: string | null | undefined): string {
  if (!isoStr) return '';
  if (!isoStr.includes('T')) return isoStr;
  const d = new Date(isoStr);
  if (Number.isNaN(d.getTime())) return isoStr;
  const parts: Record<string, string> = {};
  for (const p of BANGKOK_PARTS.formatToParts(d)) {
    parts[p.type] = p.value;
  }
  return `${parts.year}-${parts.month}-${parts.day}T${parts.hour}:${parts.minute}:${parts.second}+07:00`;
}

function toBangkokDate(isoStr: string | null | undefined): string {
  const wallClock = toBangkokWallClock(isoStr);
  return wallClock.includes('T') ? wallClock.slice(0, 10) : wallClock;
}

function parseInteger(raw: string): number | null {
  const trimmed = raw.trim();
  if (!/^-?\d+$/.test(trimmed)) return null;
  const n = Number(trimmed);
  return Number.isSafeInteger(n) ? n : null;
}

export function Campaigns(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const createWrite = keyedWrite<CampaignCreateInput, CampaignCreateOutput>(api, 'POST', '/api/campaigns');
  const saveWrite = keyedWrite<
    CampaignVersionSaveInput,
    CampaignVersionSaveOutput
  >(
    api,
    'PUT',
    (b) => `/api/campaigns/${b.campaignId}/draft`,
  );
  const simulateWrite = keyedWrite<CampaignSimulateInput, CampaignSimulateOutput>(
    api,
    'POST',
    (b) => `/api/campaigns/${b.campaignId}/simulate`,
  );
  const submitWrite = keyedWrite<CampaignSubmitInput, CampaignSubmitOutput>(
    api,
    'POST',
    (b) => `/api/campaigns/${b.campaignId}/submit`,
  );
  const pauseWrite = keyedWrite<CampaignPauseInput, CampaignPauseOutput>(
    api,
    'POST',
    (b) => `/api/campaigns/${b.campaignId}/pause`,
  );
  const resumeWrite = keyedWrite<CampaignResumeInput, CampaignResumeOutput>(
    api,
    'POST',
    (b) => `/api/campaigns/${b.campaignId}/resume`,
  );
  const newVersionWrite = keyedWrite<CampaignVersionNewInput, CampaignVersionNewOutput>(
    api,
    'POST',
    (b) => `/api/campaigns/${b.campaignId}/versions`,
  );
  const issueCouponsWrites = new Map<
    string,
    { send(body: { ruleId: string; count: number; expiresAt: string }): Promise<CouponIssueOutput>; reset(): void }
  >();

  function issueCouponsWriteFor(campaignId: string) {
    let w = issueCouponsWrites.get(campaignId);
    if (!w) {
      w = keyedWrite<{ ruleId: string; count: number; expiresAt: string }, CouponIssueOutput>(
        api,
        'POST',
        `/api/campaigns/${campaignId}/coupons`,
      );
      issueCouponsWrites.set(campaignId, w);
    }
    return w;
  }

  function resetWrites(): void {
    createWrite.reset();
    saveWrite.reset();
    simulateWrite.reset();
    submitWrite.reset();
    pauseWrite.reset();
    resumeWrite.reset();
    newVersionWrite.reset();
    for (const w of issueCouponsWrites.values()) w.reset();
  }

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
  const [createPriority, setCreatePriority] = createSignal('10');
  const [createBranches, setCreateBranches] = createSignal<string[]>(['MAIN']);
  const [createError, setCreateError] = createSignal<string | null>(null);
  const [createFieldErrors, setCreateFieldErrors] = createSignal<Record<string, string>>({});

  // M02 Detail state
  const [selectedCampaignId, setSelectedCampaignId] = createSignal<string | null>(null);
  const [campaignDetail, setCampaignDetail] = createSignal<CampaignDetail | null>(null);
  const [loadingDetail, setLoadingDetail] = createSignal(false);
  const [detailError, setDetailError] = createSignal<string | null>(null);
  const [detailFieldErrors, setDetailFieldErrors] = createSignal<Record<string, string>>({});
  // #2441: id of the campaign whose detail fills the form; null until its read answers.
  const [loadedDetailId, setLoadedDetailId] = createSignal<string | null>(null);

  // Detail Form inputs
  const [formName, setFormName] = createSignal('');
  const [formBudgetThb, setFormBudgetThb] = createSignal('');
  const [formStartsAt, setFormStartsAt] = createSignal('');
  const [formEndsAt, setFormEndsAt] = createSignal('');
  const [formStackGroup, setFormStackGroup] = createSignal('DEFAULT');
  const [formPriority, setFormPriority] = createSignal('0');
  const [formBranches, setFormBranches] = createSignal<string[]>([]);
  const [formRules, setFormRules] = createSignal<RuleForm[]>([]);
  const [ruleRowErrors, setRuleRowErrors] = createSignal<RuleError[]>([]);
  const [rulesSupported, setRulesSupported] = createSignal(true);
  const [isEditedAfterSimulation, setIsEditedAfterSimulation] = createSignal(false);

  // #4842 SIM-1..SIM-6: Simulate scenario inputs
  const [simBranch, setSimBranch] = createSignal('');
  const [simCurrency, setSimCurrency] = createSignal('USD');
  const [simAmount, setSimAmount] = createSignal('100');

  // Coupon issue inputs
  const [couponRuleId, setCouponRuleId] = createSignal('');
  const [couponCount, setCouponCount] = createSignal('1');
  const [couponExpiresAt, setCouponExpiresAt] = createSignal('');

  // Issued coupons from couponIssue
  const [issuedCoupons, setIssuedCoupons] = createSignal<IssuedCouponDto[] | null>(null);

  // #2426: true while a campaign write is in flight; blocks a second write (reads stay unblocked).
  const [writing, setWriting] = createSignal(false);

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
      setLoadedDetailId(detail ? id : null);

      if (detail) {
        const v = detail.versions[0];
        if (!keepUnsaved) {
          setFormName(detail.name ?? '');
          setFormBudgetThb(v?.budgetThb ?? '');
          setFormStartsAt(toBangkokWallClock(v?.startsAt));
          setFormEndsAt(toBangkokWallClock(v?.endsAt));
          setFormStackGroup(v?.stackGroup ?? 'DEFAULT');
          setFormPriority(String(v?.priority ?? 0));
          const nextBranches = v?.branches ? [...v.branches] : [];
          setFormBranches(nextBranches);
          if (!nextBranches.includes(simBranch())) {
            setSimBranch(nextBranches[0] ?? '');
          }
          const storedRules = v?.rules ?? [];
          const supported = storedRules.every((r) => isKnownBenefitType(r.benefitType));
          setRulesSupported(supported);
          setFormRules(supported ? storedRules.map(ruleFormOf) : []);
          setRuleRowErrors([]);
          setDetailFieldErrors({});
          setIsEditedAfterSimulation(false);
        }
        if (v?.rules && v.rules.length > 0) {
          setCouponRuleId(v.rules[0].id);
        } else {
          setCouponRuleId('');
        }
        if (v?.endsAt) {
          setCouponExpiresAt(toBangkokDate(v.endsAt));
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

  // #2441: forget the shown campaign so its values can never be shown or sent for another one.
  // Bumping `readingDetail` drops any detail read still in flight.
  function resetDetailForm(): void {
    readingDetail++;
    setLoadingDetail(false);
    setCampaignDetail(null);
    setLoadedDetailId(null);
    setFormName('');
    setFormBudgetThb('');
    setFormStartsAt('');
    setFormEndsAt('');
    setFormStackGroup('DEFAULT');
    setFormPriority('0');
    setFormBranches([]);
    setFormRules([]);
    setRuleRowErrors([]);
    setDetailFieldErrors({});
    setRulesSupported(true);
    setIsEditedAfterSimulation(false);
    setSimBranch('');
    setSimCurrency('USD');
    setSimAmount('100');
    setCouponRuleId('');
    setCouponCount('1');
    setCouponExpiresAt('');
    setIssuedCoupons(null);
  }

  let lastBranch: string | undefined = undefined;
  createEffect(() => {
    if (props.shown && !props.shown()) return;
    const branch = props.branch?.();
    const p = props.period();
    if (lastBranch !== undefined && branch !== lastBranch) {
      setSelectedCampaignId(null);
      resetDetailForm();
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

  const validScenario = () => {
    const branchId = simBranch();
    if (!branchId || !formBranches().includes(branchId)) return null;
    const currency = simCurrency().trim().toUpperCase();
    if (!/^[A-Z]{3}$/.test(currency)) return null;
    const amount = simAmount().trim();
    if (!/^\d+(?:\.\d+)?$/.test(amount) || !(Number(amount) > 0)) return null;
    return { branchId, currency, amount };
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
    setCreatePriority('10');
    setCreateBranches(['MAIN']);
    setCreateError(null);
    setCreateFieldErrors({});
    resetWrites();
  }

  function openCampaign(campaignId: string) {
    setIsCreating(false);
    resetDetailForm();
    setSelectedCampaignId(campaignId);
    resetWrites();
    void loadCampaignDetail(campaignId, false);
  }

  // #2441: Save / Issue coupons only send once the selected campaign's own detail is loaded.
  const detailReady = () => {
    const id = selectedCampaignId();
    return id !== null && loadedDetailId() === id && campaignDetail() !== null;
  };

  function handleInputEdit(setter: (val: string) => void, val: string) {
    setter(val);
    setIsEditedAfterSimulation(true);
  }

  function applyCreateError(err: unknown) {
    const fe = campaignFieldError(err);
    if (fe.field) {
      setCreateFieldErrors({ [fe.field]: fe.message });
    } else {
      setCreateError(fe.message);
    }
  }

  function applyDetailError(err: unknown) {
    const fe = campaignFieldError(err);
    if (fe.field) {
      setDetailFieldErrors({ [fe.field]: fe.message });
    } else {
      setDetailError(fe.message);
    }
  }

  async function handleCreate() {
    setCreateError(null);
    setCreateFieldErrors({});
    const rawBranches = createBranches() as unknown;
    const branches: string[] = Array.isArray(rawBranches)
      ? rawBranches.map(String)
      : typeof rawBranches === 'string' && rawBranches.length > 0
        ? [rawBranches]
        : [];
    const fieldErrs: Record<string, string> = {};
    if (createName().trim() === '') {
      fieldErrs.name = localFieldError('name');
    }
    const startsAt = toBangkokStart(createStartsAt());
    const endsAt = toBangkokEnd(createEndsAt());
    const startMs = startsAt ? Date.parse(startsAt) : Number.NaN;
    const endMs = endsAt ? Date.parse(endsAt) : Number.NaN;
    if (Number.isNaN(startMs)) {
      fieldErrs.startsAt = localFieldError('startsAt');
    }
    if (Number.isNaN(endMs)) {
      fieldErrs.endsAt = localFieldError('endsAt');
    } else if (!Number.isNaN(startMs) && endMs <= startMs) {
      fieldErrs.endsAt = localFieldError('endsAt');
    }
    if (!checkDecimal(createBudgetThb(), BUDGET_BOUND).ok) {
      fieldErrs.budgetThb = localFieldError('budgetThb');
    }
    if (branches.length === 0) {
      fieldErrs.branches = localFieldError('branches');
    }
    if (Object.keys(fieldErrs).length > 0) {
      setCreateFieldErrors(fieldErrs);
      return;
    }
    const rawPriority = createPriority().trim();
    if (/^-?\d+$/.test(rawPriority) && (BigInt(rawPriority) < 0n || BigInt(rawPriority) > 2147483647n)) {
      setCreateFieldErrors({ priority: localFieldError('priority') });
      return;
    }
    const priority = parseInteger(createPriority());
    if (priority === null) {
      setCreateError(t('error.validationFailed'));
      return;
    }
    const body: CampaignCreateInput = {
      name: createName(),
      startsAt,
      endsAt,
      budgetThb: createBudgetThb(),
      stackGroup: createStackGroup() || 'DEFAULT',
      priority,
      branches,
    };
    if (writing()) return;
    setWriting(true);
    try {
      const res = await createWrite.send(body);
      setIsCreating(false);
      if (res?.id) {
        openCampaign(res.id);
      } else {
        void loadList(props.branch?.(), props.period().from, props.period().to);
      }
    } catch (err: unknown) {
      applyCreateError(err);
    } finally {
      setWriting(false);
    }
  }

  // #2438: a write's answer applies only while the campaign it was sent for is still shown.
  const stillShowing = (campaignId: string) => selectedCampaignId() === campaignId;

  async function handleSave() {
    const campaignId = selectedCampaignId();
    if (!campaignId || !detailReady()) return;
    setDetailError(null);
    setDetailFieldErrors({});
    const rawBranches = formBranches() as unknown;
    const branches: string[] = Array.isArray(rawBranches)
      ? rawBranches.map(String)
      : typeof rawBranches === 'string' && rawBranches.length > 0
        ? [rawBranches]
        : [];
    const fieldErrs: Record<string, string> = {};
    const startsAt = toBangkokStart(formStartsAt());
    const endsAt = toBangkokEnd(formEndsAt());
    const startMs = startsAt ? Date.parse(startsAt) : Number.NaN;
    const endMs = endsAt ? Date.parse(endsAt) : Number.NaN;
    if (Number.isNaN(startMs)) {
      fieldErrs.startsAt = localFieldError('startsAt');
    }
    if (Number.isNaN(endMs)) {
      fieldErrs.endsAt = localFieldError('endsAt');
    } else if (!Number.isNaN(startMs) && endMs <= startMs) {
      fieldErrs.endsAt = localFieldError('endsAt');
    }
    if (!checkDecimal(formBudgetThb(), BUDGET_BOUND).ok) {
      fieldErrs.budgetThb = localFieldError('budgetThb');
    }
    if (branches.length === 0) {
      fieldErrs.branches = localFieldError('branches');
    }
    const errs = rulesSupported() ? formRules().map(ruleErrors) : [];
    if (rulesSupported()) {
      setRuleRowErrors(errs);
    }
    if (Object.keys(fieldErrs).length > 0) {
      setDetailFieldErrors(fieldErrs);
      return;
    }
    if (errs.some((e) => e.calculation || e.caps)) {
      return;
    }
    const rawPriority = formPriority().trim();
    if (/^-?\d+$/.test(rawPriority) && (BigInt(rawPriority) < 0n || BigInt(rawPriority) > 2147483647n)) {
      setDetailFieldErrors({ priority: localFieldError('priority') });
      return;
    }
    const priority = parseInteger(formPriority());
    if (priority === null) {
      setDetailError(t('error.validationFailed'));
      return;
    }
    const body: CampaignVersionSaveInput = {
      campaignId,
      startsAt,
      endsAt,
      budgetThb: formBudgetThb(),
      stackGroup: formStackGroup(),
      priority,
      branches,
      ...(rulesSupported() ? { rules: formRules().map(ruleInputOf) } : {}),
    };
    if (writing()) return;
    setWriting(true);
    try {
      await saveWrite.send(body);
      if (!stillShowing(campaignId)) return;
      setIsEditedAfterSimulation(false);
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadCampaignDetail(campaignId, true);
      }
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handleSimulate() {
    const campaignId = selectedCampaignId();
    const scenario = validScenario();
    if (!campaignId || !scenario) return;
    const body: CampaignSimulateInput = {
      campaignId,
      scenarios: [scenario],
    };
    if (writing()) return;
    setWriting(true);
    setDetailError(null);
    setDetailFieldErrors({});
    try {
      await simulateWrite.send(body);
      if (!stillShowing(campaignId)) return;
      setIsEditedAfterSimulation(false);
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handleSubmit() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignSubmitInput = {
      campaignId,
    };
    if (writing()) return;
    setWriting(true);
    setDetailError(null);
    setDetailFieldErrors({});
    try {
      await submitWrite.send(body);
      if (!stillShowing(campaignId)) return;
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handlePause() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignPauseInput = {
      campaignId,
    };
    if (writing()) return;
    setWriting(true);
    setDetailError(null);
    setDetailFieldErrors({});
    try {
      await pauseWrite.send(body);
      if (!stillShowing(campaignId)) return;
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handleResume() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignResumeInput = {
      campaignId,
    };
    if (writing()) return;
    setWriting(true);
    setDetailError(null);
    setDetailFieldErrors({});
    try {
      await resumeWrite.send(body);
      if (!stillShowing(campaignId)) return;
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handleNewVersion() {
    const campaignId = selectedCampaignId();
    if (!campaignId) return;
    const body: CampaignVersionNewInput = {
      campaignId,
    };
    if (writing()) return;
    setWriting(true);
    setDetailError(null);
    setDetailFieldErrors({});
    try {
      await newVersionWrite.send(body);
      if (!stillShowing(campaignId)) return;
      await loadCampaignDetail(campaignId, false);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
    }
  }

  async function handleIssueCoupons() {
    const campaignId = selectedCampaignId();
    if (!campaignId || !detailReady()) return;
    setDetailError(null);
    setDetailFieldErrors({});
    const expiresAt = toBangkokEnd(couponExpiresAt().trim());
    if (!expiresAt || Number.isNaN(Date.parse(expiresAt))) {
      setDetailFieldErrors({ expiresAt: localFieldError('expiresAt') });
      return;
    }
    const rawCount = couponCount().trim();
    if (/^\d+$/.test(rawCount) && BigInt(rawCount) > 1000n) {
      setDetailFieldErrors({ count: localFieldError('count') });
      return;
    }
    const count = parseInteger(couponCount());
    if (count === null || count < 1) {
      setDetailError(t('error.validationFailed'));
      return;
    }
    const ruleId = couponRuleId() || currentVersion()?.rules?.[0]?.id || '';
    const body = {
      ruleId,
      count,
      expiresAt,
    };
    if (writing()) return;
    setWriting(true);
    try {
      const res = await issueCouponsWriteFor(campaignId).send(body);
      if (!stillShowing(campaignId)) return;
      setIssuedCoupons(res?.coupons ?? []);
    } catch (err: unknown) {
      if (!stillShowing(campaignId)) return;
      applyDetailError(err);
    } finally {
      setWriting(false);
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
                  disabled={writing()}
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
                <Show when={createFieldErrors().name}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="name">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                <Show when={createFieldErrors().startsAt}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="startsAt">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                <Show when={createFieldErrors().endsAt}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="endsAt">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                <Show when={createFieldErrors().budgetThb}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="budgetThb">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                  min="0"
                  max="2147483647"
                  value={createPriority()}
                  onInput={(e) => setCreatePriority(e.currentTarget.value)}
                  onChange={(e) => setCreatePriority(e.currentTarget.value)}
                />
                <Show when={createFieldErrors().priority}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="priority">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                <Show when={createFieldErrors().branches}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="branches">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                  resetDetailForm();
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
                disabled={writing() || !detailReady()}
                onClick={handleSave}
              >
                {t('save')}
              </button>

              <div class="xc-campaign-sim-row">
                <Show
                  when={formBranches().length > 0}
                  fallback={<p class="xc-campaign-sim-note">{t('simNeedsBranch')}</p>}
                >
                  <label class="xc-campaign-sim-field">
                    <span>{t('simBranch')}</span>
                    <select
                      data-testid="sim-branch"
                      class="xc-select"
                      value={simBranch()}
                      onInput={(e) => setSimBranch(e.currentTarget.value)}
                      onChange={(e) => setSimBranch(e.currentTarget.value)}
                    >
                      <For each={formBranches()}>
                        {(b) => <option value={b}>{b}</option>}
                      </For>
                    </select>
                  </label>
                </Show>

                <label class="xc-campaign-sim-field">
                  <span>{t('simCurrency')}</span>
                  <input
                    data-testid="sim-currency"
                    class="xc-input"
                    value={simCurrency()}
                    onInput={(e) => setSimCurrency(e.currentTarget.value.toUpperCase())}
                    onChange={(e) => setSimCurrency(e.currentTarget.value.toUpperCase())}
                  />
                </label>

                <label class="xc-campaign-sim-field">
                  <span>{t('simAmount')}</span>
                  <input
                    data-testid="sim-amount"
                    class="xc-input"
                    value={simAmount()}
                    onInput={(e) => setSimAmount(e.currentTarget.value)}
                    onChange={(e) => setSimAmount(e.currentTarget.value)}
                  />
                  <Show when={detailFieldErrors().amount}>
                    {(msg) => (
                      <div class="xc-field-error" data-field-error="amount">
                        {msg()}
                      </div>
                    )}
                  </Show>
                </label>
              </div>

              <button
                type="button"
                class="xc-btn-secondary"
                disabled={writing() || !validScenario()}
                onClick={handleSimulate}
              >
                {t('simulate')}
              </button>

              <button
                type="button"
                class="xc-btn-success"
                disabled={writing() || isSubmitDisabled()}
                onClick={handleSubmit}
              >
                {t('submit')}
              </button>

              <Show when={currentStatus() === 'ACTIVE'}>
                <button
                  type="button"
                  class="xc-btn-warning"
                  disabled={writing()}
                  onClick={handlePause}
                >
                  {t('pause')}
                </button>
              </Show>

              <Show when={currentStatus() === 'PAUSED'}>
                <button
                  type="button"
                  class="xc-btn-success"
                  disabled={writing()}
                  onClick={handleResume}
                >
                  {t('resume')}
                </button>
              </Show>

              <Show when={currentStatus() === 'ACTIVE' || currentStatus() === 'PAUSED'}>
                <button
                  type="button"
                  class="xc-btn-secondary"
                  disabled={writing()}
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
              <Show when={detailFieldErrors().name}>
                {(msg) => (
                  <div class="xc-field-error" data-field-error="name">
                    {msg()}
                  </div>
                )}
              </Show>
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
              <Show when={detailFieldErrors().budgetThb}>
                {(msg) => (
                  <div class="xc-field-error" data-field-error="budgetThb">
                    {msg()}
                  </div>
                )}
              </Show>
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
              <Show when={detailFieldErrors().startsAt}>
                {(msg) => (
                  <div class="xc-field-error" data-field-error="startsAt">
                    {msg()}
                  </div>
                )}
              </Show>
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
              <Show when={detailFieldErrors().endsAt}>
                {(msg) => (
                  <div class="xc-field-error" data-field-error="endsAt">
                    {msg()}
                  </div>
                )}
              </Show>
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
                min="0"
                max="2147483647"
                value={formPriority()}
                onInput={(e) => handleInputEdit(setFormPriority, e.currentTarget.value)}
                onChange={(e) => handleInputEdit(setFormPriority, e.currentTarget.value)}
              />
              <Show when={detailFieldErrors().priority}>
                {(msg) => (
                  <div class="xc-field-error" data-field-error="priority">
                    {msg()}
                  </div>
                )}
              </Show>
            </div>

            <Show when={detailFieldErrors().branches}>
              {(msg) => (
                <div class="xc-field-error" data-field-error="branches">
                  {msg()}
                </div>
              )}
            </Show>
          </div>

          <CampaignRulesEditor
            rules={formRules()}
            readOnly={currentStatus() !== 'DRAFT' || !rulesSupported()}
            errors={ruleRowErrors()}
            rulesError={detailFieldErrors().rules}
            onChange={(next) => {
              setFormRules(next);
              setRuleRowErrors([]);
              setIsEditedAfterSimulation(true);
            }}
          />

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
                  data-field="count"
                  class="xc-input"
                  min="1"
                  max="1000"
                  value={couponCount()}
                  onInput={(e) => setCouponCount(e.currentTarget.value)}
                  onChange={(e) => setCouponCount(e.currentTarget.value)}
                />
                <Show when={detailFieldErrors().count}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="count">
                      {msg()}
                    </div>
                  )}
                </Show>
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
                <Show when={detailFieldErrors().expiresAt}>
                  {(msg) => (
                    <div class="xc-field-error" data-field-error="expiresAt">
                      {msg()}
                    </div>
                  )}
                </Show>
              </div>
            </div>

            <button
              type="button"
              class="xc-btn-secondary"
              disabled={writing() || !detailReady()}
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
