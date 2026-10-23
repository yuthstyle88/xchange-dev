import type { CampaignRuleDetail } from '../../../shared/api/generated/campaign';
import type { CampaignRuleInput as WireRuleInput } from '../../../shared/api/generated/campaign_version_save';
import { MONEY, checkDecimal } from '../../../shared/ui/bounded';

export type BenefitType = 'RATE_IMPROVEMENT' | 'FEE_WAIVER';

// Narrows the generated wire CampaignRuleInput for the campaign screen editor.
export type CampaignRuleInput = WireRuleInput & {
  benefitType: BenefitType;
  eligibility: Record<string, unknown>;
  calculation: Record<string, string>;
  caps: {
    perTransactionMaxThb?: string;
    perCustomerUses?: number;
    perCustomerPeriodDays?: number;
  };
};

export type RuleForm = {
  benefitType: BenefitType;
  calcMode: string;
  calcValue: string;
  currencies: string;
  shopSide: '' | 'BUY' | 'SELL';
  minThb: string;
  perTransactionMaxThb: string;
  perCustomerUses: string;
  perCustomerPeriodDays: string;
  requiresCoupon: boolean;
  kept: Record<string, unknown>;
  keptCalculation?: Record<string, string>;
};

export type RuleError = {
  calculation?: true;
  caps?: true;
};

const RATE_IMPROVEMENT_MODES = ['delta', 'percent'] as const;
const FEE_WAIVER_MODES = ['percent', 'amount', 'fixed'] as const;
const EDITOR_ELIGIBILITY_KEYS = new Set(['currencies', 'shopSide', 'minThb']);
const DECIMAL_8_RE = /^\d+(?:\.\d{1,8})?$/;
const POS_INT_RE = /^[1-9]\d*$/;

export function isKnownBenefitType(benefitType: string): benefitType is BenefitType {
  return benefitType === 'RATE_IMPROVEMENT' || benefitType === 'FEE_WAIVER';
}

export function calcModesFor(benefitType: BenefitType): readonly string[] {
  return benefitType === 'FEE_WAIVER' ? FEE_WAIVER_MODES : RATE_IMPROVEMENT_MODES;
}

export function emptyRuleForm(): RuleForm {
  return {
    benefitType: 'RATE_IMPROVEMENT',
    calcMode: 'delta',
    calcValue: '',
    currencies: '',
    shopSide: '',
    minThb: '',
    perTransactionMaxThb: '',
    perCustomerUses: '',
    perCustomerPeriodDays: '',
    requiresCoupon: false,
    kept: {},
  };
}

function asRecord(val: unknown): Record<string, unknown> {
  if (val !== null && typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>;
  }
  return {};
}

function strOrEmpty(val: unknown): string {
  if (typeof val === 'string') return val;
  if (typeof val === 'number' && Number.isFinite(val)) return String(val);
  return '';
}

export function ruleFormOf(stored: CampaignRuleDetail): RuleForm {
  const benefitType: BenefitType = stored.benefitType === 'FEE_WAIVER' ? 'FEE_WAIVER' : 'RATE_IMPROVEMENT';
  const knownModes = calcModesFor(benefitType);

  const calcObj = asRecord(stored.calculation);
  const calcEntries: Array<[string, string]> = [];
  for (const [k, v] of Object.entries(calcObj)) {
    if (typeof v === 'string' || (typeof v === 'number' && Number.isFinite(v))) {
      calcEntries.push([k, String(v)]);
    }
  }

  let calcMode = knownModes[0]!;
  let calcValue = '';
  let keptCalculation: Record<string, string> | undefined = undefined;

  if (calcEntries.length === 1 && knownModes.includes(calcEntries[0]![0])) {
    calcMode = calcEntries[0]![0];
    calcValue = calcEntries[0]![1];
  } else if (calcEntries.length > 0) {
    calcMode = calcEntries[0]![0];
    calcValue = calcEntries[0]![1];
    keptCalculation = Object.fromEntries(calcEntries);
  }

  const eligObj = asRecord(stored.eligibility);
  const currencies = Array.isArray(eligObj.currencies)
    ? eligObj.currencies.map((c) => String(c).trim()).filter(Boolean).join(', ')
    : '';
  const shopSide: '' | 'BUY' | 'SELL' =
    eligObj.shopSide === 'BUY' || eligObj.shopSide === 'SELL' ? eligObj.shopSide : '';
  const minThb = strOrEmpty(eligObj.minThb);

  const kept: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(eligObj)) {
    if (!EDITOR_ELIGIBILITY_KEYS.has(k)) {
      kept[k] = v;
    }
  }

  const capsObj = asRecord(stored.caps);

  const form: RuleForm = {
    benefitType,
    calcMode,
    calcValue,
    currencies,
    shopSide,
    minThb,
    perTransactionMaxThb: strOrEmpty(capsObj.perTransactionMaxThb),
    perCustomerUses: strOrEmpty(capsObj.perCustomerUses),
    perCustomerPeriodDays: strOrEmpty(capsObj.perCustomerPeriodDays),
    requiresCoupon: Boolean(stored.requiresCoupon),
    kept,
  };
  if (keptCalculation !== undefined) {
    form.keptCalculation = keptCalculation;
  }
  return form;
}

export function ruleInputOf(form: RuleForm): CampaignRuleInput {
  const calculation: Record<string, string> = form.keptCalculation
    ? { ...form.keptCalculation }
    : { [form.calcMode]: form.calcValue.trim() };

  const eligibility: Record<string, unknown> = { ...form.kept };
  const currencies = form.currencies
    .split(',')
    .map((c) => c.trim().toUpperCase())
    .filter((c) => c.length > 0);
  if (currencies.length > 0) {
    eligibility.currencies = currencies;
  }
  if (form.shopSide === 'BUY' || form.shopSide === 'SELL') {
    eligibility.shopSide = form.shopSide;
  }
  const minThb = form.minThb.trim();
  if (minThb !== '') {
    eligibility.minThb = minThb;
  }

  const caps: CampaignRuleInput['caps'] = {};
  const maxThb = form.perTransactionMaxThb.trim();
  if (maxThb !== '') {
    caps.perTransactionMaxThb = maxThb;
  }
  const uses = form.perCustomerUses.trim();
  if (uses !== '') {
    caps.perCustomerUses = Number(uses);
  }
  const periodDays = form.perCustomerPeriodDays.trim();
  if (periodDays !== '') {
    caps.perCustomerPeriodDays = Number(periodDays);
  }

  return {
    benefitType: form.benefitType,
    calculation,
    eligibility,
    caps,
    requiresCoupon: form.requiresCoupon,
  };
}

function isValidCalculation(form: RuleForm): boolean {
  if (form.keptCalculation) {
    return true;
  }
  const raw = form.calcValue.trim();
  if (
    form.benefitType === 'FEE_WAIVER' &&
    (form.calcMode === 'amount' || form.calcMode === 'fixed')
  ) {
    return checkDecimal(raw, MONEY).ok && Number(raw) > 0;
  }
  if (!DECIMAL_8_RE.test(raw)) {
    return false;
  }
  const n = Number(raw);
  if (!Number.isFinite(n)) {
    return false;
  }
  if (form.benefitType === 'RATE_IMPROVEMENT') {
    if (form.calcMode === 'delta') return n > 0;
    if (form.calcMode === 'percent') return n > 0 && n < 1;
    return false;
  }
  if (form.benefitType === 'FEE_WAIVER') {
    if (form.calcMode === 'percent') return n > 0 && n <= 1;
    return false;
  }
  return false;
}

function isValidCaps(form: RuleForm): boolean {
  const maxThb = form.perTransactionMaxThb.trim();
  if (maxThb !== '' && (!DECIMAL_8_RE.test(maxThb) || Number(maxThb) <= 0)) {
    return false;
  }
  const uses = form.perCustomerUses.trim();
  if (uses !== '' && !POS_INT_RE.test(uses)) {
    return false;
  }
  const periodDays = form.perCustomerPeriodDays.trim();
  if (periodDays !== '' && !POS_INT_RE.test(periodDays)) {
    return false;
  }
  return true;
}

export function ruleErrors(form: RuleForm): RuleError {
  const err: RuleError = {};
  if (!isValidCalculation(form)) {
    err.calculation = true;
  }
  if (!isValidCaps(form)) {
    err.caps = true;
  }
  return err;
}
