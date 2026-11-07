import { createSignal, For, onMount, Show, type Accessor, type JSX } from 'solid-js';
import { ApiError, type ApiClient } from '../../shared/api/client';
import type { Me } from '../../shared/api/generated/common';
import type { BackdateCurrenciesData, BackdateCurrencyRow } from '../../shared/api/generated/backdate_currencies';
import type { StaffCustomerRow, StaffCustomersFilter } from '../../shared/api/generated/staff_customers';
import type { TransactionBackdateInput } from '../../shared/api/generated/transaction_backdate';
import type { TransactionDto } from '../../shared/api/generated/transactions';
import { keyedWrite } from '../../shared/api/keyed-write';
import { screenT } from '../../shared/i18n';
import { wordedError } from '../../shared/i18n/worded-error';
import { sharedApi, sharedSession } from '../../shared/session/session';
import '../desk/desk.css';
import { METHODS } from '../desk/steps/payment/channels';
import { en, th } from './i18n';
import '../../shared/ui/card.css';
import './backdated.css';

const t = screenT('desk-backdated', { th, en });

const MAX_FILE_BYTES = 15_728_640;
const ACCEPTED_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'application/pdf'];
const ACCEPT = '.jpg,.jpeg,.png,.webp,.pdf,image/jpeg,image/png,image/webp,application/pdf';

interface ParsedDecimal {
  value: bigint;
  scale: number;
}

function parseNonNegativeDecimal(raw: string): ParsedDecimal | null {
  const s = raw.trim();
  if (!/^\d+(?:\.\d+)?$/.test(s)) return null;
  const dot = s.indexOf('.');
  if (dot === -1) {
    return { value: BigInt(s), scale: 0 };
  }
  const intPart = s.slice(0, dot);
  const fracPart = s.slice(dot + 1);
  return { value: BigInt(intPart + fracPart), scale: fracPart.length };
}

function formatSatang(satang: bigint): string {
  const whole = satang / 100n;
  const frac = (satang % 100n).toString().padStart(2, '0');
  return `${whole}.${frac}`;
}

/**
 * `quantity × rate ÷ unitSize`, rounded HALF_UP (away from zero) to 2 decimal places (`backdated-keying.md` §6, §13).
 */
export function computeThb(quantity: string, rate: string, unitSize?: string): string | null {
  if (!unitSize) return null;
  const q = parseNonNegativeDecimal(quantity);
  const r = parseNonNegativeDecimal(rate);
  const u = parseNonNegativeDecimal(unitSize);
  if (!q || !r || !u || q.value <= 0n || r.value <= 0n || u.value <= 0n) {
    return null;
  }
  const expDiff = 2 + u.scale - (q.scale + r.scale);
  let num = q.value * r.value;
  let den = u.value;
  if (expDiff >= 0) {
    num *= 10n ** BigInt(expDiff);
  } else {
    den *= 10n ** BigInt(-expDiff);
  }
  const quot = num / den;
  const rem = num % den;
  const satang = rem * 2n >= den ? quot + 1n : quot;
  return formatSatang(satang);
}

export function computeSettleThb(
  thbAmount: string | null,
  feeThb: string,
  side: 'BUY' | 'SELL',
): string | null {
  if (!thbAmount) return null;
  const thb = parseNonNegativeDecimal(thbAmount);
  const feeStr = feeThb.trim() === '' ? '0' : feeThb.trim();
  const fee = parseNonNegativeDecimal(feeStr);
  if (!thb || !fee || fee.scale > 2) return null;
  const thbSatang = thb.value * 10n ** BigInt(2 - thb.scale);
  const feeSatang = fee.value * 10n ** BigInt(2 - fee.scale);
  const settle = side === 'BUY' ? thbSatang - feeSatang : thbSatang + feeSatang;
  if (settle <= 0n) return null;
  return formatSatang(settle);
}

/**
 * Formats a Bangkok local date-time (`YYYY-MM-DDTHH:mm` or `YYYY-MM-DDTHH:mm:ss`) as RFC 3339 with `+07:00`.
 */
export function toBangkokRfc3339(paidAt: string): string {
  const s = paidAt.trim();
  if (!s) return '';
  if (/(?:Z|[+-]\d{2}:\d{2})$/i.test(s)) return s;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(s)) return `${s}:00+07:00`;
  if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}$/.test(s)) return `${s}+07:00`;
  if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/.test(s)) return `${s.replace(' ', 'T')}:00+07:00`;
  if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/.test(s)) return `${s.replace(' ', 'T')}+07:00`;
  return s;
}

function fileValid(file: File): boolean {
  const name = file.name.toLowerCase();
  const heic = file.type === 'image/heic' || file.type === 'image/heif' || /\.(heic|heif)$/.test(name);
  const known = ACCEPTED_TYPES.includes(file.type) || /\.(jpe?g|png|webp|pdf)$/.test(name);
  return !heic && known && file.size > 0 && file.size <= MAX_FILE_BYTES;
}

function makeUploadWrite(api: ApiClient, slot: 'passport' | 'slip') {
  let activeFile: File | undefined;
  const w = keyedWrite<{ slot: string; name: string; size: number; type: string }, string>(
    {
      command: async (_method: string, path: string, _body: unknown, key: string) => {
        const file = activeFile!;
        const answer = await api.postBytes<{ uploadKey?: unknown }>(
          path,
          file,
          file.type || 'application/octet-stream',
          key,
        );
        if (!answer || typeof answer.uploadKey !== 'string') {
          throw new Error('no upload key');
        }
        return answer.uploadKey;
      },
    } as unknown as ApiClient,
    'POST',
    '/api/upload',
  );
  return {
    send(file: File): Promise<string> {
      activeFile = file;
      w.reset();
      return w.send({ slot, name: file.name, size: file.size, type: file.type });
    },
  };
}

export function BackdatedScreen(props: { api?: ApiClient; me?: Accessor<Me | undefined> } = {}): JSX.Element {
  const api = props.api ?? sharedApi();
  const me = props.me ?? sharedSession().me;

  const [currencies, setCurrencies] = createSignal<BackdateCurrencyRow[]>([]);
  const [customerSearch, setCustomerSearch] = createSignal('');
  const [customers, setCustomers] = createSignal<StaffCustomerRow[]>([]);
  const [customerId, setCustomerId] = createSignal('');
  const [side, setSide] = createSignal<'BUY' | 'SELL'>('BUY');
  const [currency, setCurrency] = createSignal('USD');
  const [quantity, setQuantity] = createSignal('');
  const [rate, setRate] = createSignal('');
  const [feeThb, setFeeThb] = createSignal('0');
  const [expectedSettleThb, setExpectedSettleThb] = createSignal('');
  const [paidAt, setPaidAt] = createSignal('');
  const [method, setMethod] = createSignal('Cash');
  const [reference, setReference] = createSignal('');
  const [account, setAccount] = createSignal('');
  const [bankFeeThb, setBankFeeThb] = createSignal('0');
  const [slipFile, setSlipFile] = createSignal<File | undefined>();
  const [passportFile, setPassportFile] = createSignal<File | undefined>();
  const [notes, setNotes] = createSignal('');

  const [submitting, setSubmitting] = createSignal(false);
  const [saved, setSaved] = createSignal<TransactionDto | undefined>();
  const [error, setError] = createSignal<string | undefined>();
  const [fieldErrors, setFieldErrors] = createSignal<Record<string, string>>({});

  const backdateWrite = keyedWrite<TransactionBackdateInput, TransactionDto>(
    api,
    'POST',
    '/api/transactions/backdated',
  );
  const passportUpload = makeUploadWrite(api, 'passport');
  const slipUpload = makeUploadWrite(api, 'slip');

  onMount(() => {
    const branch = me()?.branch ?? '';
    void api
      .view<BackdateCurrenciesData>('backdate_currencies', branch ? { branch } : {})
      .then((res) => {
        const list = res.data?.currencies ?? [];
        setCurrencies(list);
        if (list.length > 0 && !list.some((r) => r.code === currency())) {
          setCurrency(list[0]!.code);
        }
      })
      .catch(() => undefined);
  });

  const unitSize = (): string | undefined => currencies().find((r) => r.code === currency())?.unitSize;
  const computedThb = () => {
    const u = unitSize();
    return u !== undefined ? computeThb(quantity(), rate(), u) : null;
  };
  const settleThb = () => computeSettleThb(computedThb(), feeThb(), side());

  function clearField(field: string): void {
    setFieldErrors((prev) => {
      if (!(field in prev)) return prev;
      const next = { ...prev };
      delete next[field];
      return next;
    });
  }

  function handlePaidAtInput(val: string): void {
    setPaidAt(val);
    clearField('payment.paidAt');
  }

  async function searchCustomers(): Promise<void> {
    const q = customerSearch().trim();
    if (!q) {
      setCustomers([]);
      return;
    }
    try {
      const res = await api.view<StaffCustomerRow[], StaffCustomersFilter>('staff_customers', {
        filter: { q },
      });
      setCustomers(res.data ?? []);
    } catch (err) {
      setError(wordedError(err));
    }
  }

  async function onSubmit(ev: Event): Promise<void> {
    ev.preventDefault();
    if (submitting()) return;
    setSubmitting(true);
    setError(undefined);
    setFieldErrors({});
    setSaved(undefined);

    try {
      let passportKey: string | undefined;
      const pFile = passportFile();
      if (pFile) {
        passportKey = await passportUpload.send(pFile);
      }
      let slipKey: string | undefined;
      const sFile = slipFile();
      if (sFile) {
        slipKey = await slipUpload.send(sFile);
      }

      const body: TransactionBackdateInput = {
        customerId: customerId().trim(),
        side: side(),
        currency: currency().trim(),
        quantity: quantity().trim(),
        rate: rate().trim(),
        feeThb: feeThb().trim() === '' ? '0' : feeThb().trim(),
        ...(expectedSettleThb().trim() !== '' ? { expectedSettleThb: expectedSettleThb().trim() } : {}),
        payment: {
          method: method(),
          reference: reference(),
          account: account(),
          slip: slipKey ? { uploadKey: slipKey } : null,
          bankFeeThb: bankFeeThb().trim() === '' ? '0' : bankFeeThb().trim(),
          paidAt: toBangkokRfc3339(paidAt()),
        },
        passport: passportKey ? { uploadKey: passportKey } : null,
        notes: notes(),
      };

      backdateWrite.reset();
      const dto = await backdateWrite.send(body);
      setSaved(dto);
    } catch (err) {
      if (err instanceof ApiError && err.field) {
        const msg = err.message || wordedError(err);
        setFieldErrors({ [err.field]: msg });
      } else {
        setError(err instanceof ApiError && err.message ? err.message : wordedError(err));
      }
    } finally {
      setSubmitting(false);
    }
  }

  const fieldErrorText = (name: string) => fieldErrors()[name];

  return (
    <div class="desk-page">
      <div class="backdated-grid">
        <form class="desk-form backdated-form card" onSubmit={(e) => void onSubmit(e)}>
          <h2>{t('heading')}</h2>

          <div class="desk-field">
            <span>{t('customerSearch')}</span>
            <div class="desk-actions backdated-search">
              <input
                name="customerSearch"
                autocomplete="off"
                value={customerSearch()}
                onInput={(e) => setCustomerSearch(e.currentTarget.value)}
              />
              <button type="button" onClick={() => void searchCustomers()}>
                {t('customerSearchAction')}
              </button>
            </div>
            <Show when={customers().length > 0}>
              <div class="desk-actions">
                <For each={customers()}>
                  {(c) => (
                    <button
                      type="button"
                      data-customer-option={c.id}
                      onClick={() => {
                        setCustomerId(c.id);
                        clearField('customerId');
                      }}
                    >
                      {c.displayName} ({c.id})
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </div>

          <label class="desk-field">
            <span>{t('customerId')}</span>
            <input
              name="customerId"
              autocomplete="off"
              value={customerId()}
              onInput={(e) => {
                setCustomerId(e.currentTarget.value);
                clearField('customerId');
              }}
            />
            <Show when={fieldErrorText('customerId')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="customerId">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <label class="desk-field">
            <span>{t('side')}</span>
            <select
              name="side"
              value={side()}
              onChange={(e) => {
                setSide(e.currentTarget.value as 'BUY' | 'SELL');
                clearField('side');
              }}
            >
              <option value="BUY">{t('sideBuy')}</option>
              <option value="SELL">{t('sideSell')}</option>
            </select>
            <Show when={fieldErrorText('side')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="side">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <div class="backdated-grid3">
            <label class="desk-field">
              <span>{t('currency')}</span>
              <input
                name="currency"
                list="backdate-currencies"
                autocomplete="off"
                value={currency()}
                onInput={(e) => {
                  setCurrency(e.currentTarget.value.toUpperCase());
                  clearField('currency');
                }}
              />
              <datalist id="backdate-currencies">
                <For each={currencies()}>{(r) => <option value={r.code} />}</For>
              </datalist>
              <Show when={fieldErrorText('currency')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="currency">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>

            <label class="desk-field">
              <span>{t('quantity')}</span>
              <input
                name="quantity"
                inputMode="decimal"
                autocomplete="off"
                value={quantity()}
                onInput={(e) => {
                  setQuantity(e.currentTarget.value);
                  clearField('quantity');
                }}
              />
              <Show when={fieldErrorText('quantity')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="quantity">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>

            <label class="desk-field">
              <span>
                {unitSize() !== undefined ? `${t('rate')} (${t('unitSize')} ${unitSize()} ${currency()})` : t('rate')}
              </span>
              <input
                name="rate"
                inputMode="decimal"
                autocomplete="off"
                value={rate()}
                onInput={(e) => {
                  setRate(e.currentTarget.value);
                  clearField('rate');
                }}
              />
              <Show when={fieldErrorText('rate')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="rate">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>
          </div>

          <div class="backdated-grid2">
            <label class="desk-field">
              <span>{t('feeThb')}</span>
              <input
                name="feeThb"
                inputMode="decimal"
                autocomplete="off"
                value={feeThb()}
                onInput={(e) => {
                  setFeeThb(e.currentTarget.value);
                  clearField('feeThb');
                }}
              />
              <Show when={fieldErrorText('feeThb')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="feeThb">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>

            <label class="desk-field">
              <span>{t('expectedSettleThb')}</span>
              <input
                name="expectedSettleThb"
                inputMode="decimal"
                autocomplete="off"
                value={expectedSettleThb()}
                onInput={(e) => {
                  setExpectedSettleThb(e.currentTarget.value);
                  clearField('expectedSettleThb');
                }}
              />
              <Show when={fieldErrorText('expectedSettleThb')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="expectedSettleThb">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>
          </div>

          <label class="desk-field">
            <span>{t('paidAt')}</span>
            <input
              name="paidAt"
              type="datetime-local"
              value={paidAt()}
              onInput={(e) => handlePaidAtInput(e.currentTarget.value)}
            />
            <Show when={fieldErrorText('payment.paidAt')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="payment.paidAt">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <label class="desk-field">
            <span>{t('method')}</span>
            <select
              name="method"
              value={method()}
              onChange={(e) => {
                setMethod(e.currentTarget.value);
                clearField('payment.method');
              }}
            >
              <For each={METHODS}>{(m) => <option value={m}>{m}</option>}</For>
            </select>
            <Show when={fieldErrorText('payment.method')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="payment.method">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <div class="backdated-grid2">
            <label class="desk-field">
              <span>{t('reference')}</span>
              <input
                name="reference"
                autocomplete="off"
                maxLength={100}
                value={reference()}
                onInput={(e) => {
                  setReference(e.currentTarget.value);
                  clearField('payment.reference');
                }}
              />
              <Show when={fieldErrorText('payment.reference')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="payment.reference">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>

            <label class="desk-field">
              <span>{t('account')}</span>
              <input
                name="account"
                autocomplete="off"
                maxLength={100}
                value={account()}
                onInput={(e) => {
                  setAccount(e.currentTarget.value);
                  clearField('payment.account');
                }}
              />
              <Show when={fieldErrorText('payment.account')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="payment.account">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>
          </div>

          <label class="desk-field">
            <span>{t('bankFeeThb')}</span>
            <input
              name="bankFeeThb"
              inputMode="decimal"
              autocomplete="off"
              value={bankFeeThb()}
              onInput={(e) => {
                setBankFeeThb(e.currentTarget.value);
                clearField('payment.bankFeeThb');
              }}
            />
            <Show when={fieldErrorText('payment.bankFeeThb')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="payment.bankFeeThb">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <div class="backdated-grid2">
            <label class="desk-field">
              <span>{t('slip')}</span>
              <input
                type="file"
                name="slip"
                accept={ACCEPT}
                onChange={(e) => {
                  const f = e.currentTarget.files?.[0];
                  if (!f) return;
                  if (!fileValid(f)) {
                    setFieldErrors((prev) => ({ ...prev, 'payment.slip': t('fileInvalid') }));
                    e.currentTarget.value = '';
                    return;
                  }
                  clearField('payment.slip');
                  setSlipFile(f);
                }}
              />
              <Show when={slipFile()}>
                {(f) => (
                  <span class="desk-file">
                    {f().name}{' '}
                    <button type="button" onClick={() => setSlipFile(undefined)}>
                      {t('removeFile')}
                    </button>
                  </span>
                )}
              </Show>
              <Show when={fieldErrorText('payment.slip')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="payment.slip">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>

            <label class="desk-field">
              <span>{t('passport')}</span>
              <input
                type="file"
                name="passport"
                accept={ACCEPT}
                onChange={(e) => {
                  const f = e.currentTarget.files?.[0];
                  if (!f) return;
                  if (!fileValid(f)) {
                    setFieldErrors((prev) => ({ ...prev, passport: t('fileInvalid') }));
                    e.currentTarget.value = '';
                    return;
                  }
                  clearField('passport');
                  setPassportFile(f);
                }}
              />
              <Show when={passportFile()}>
                {(f) => (
                  <span class="desk-file">
                    {f().name}{' '}
                    <button type="button" onClick={() => setPassportFile(undefined)}>
                      {t('removeFile')}
                    </button>
                  </span>
                )}
              </Show>
              <Show when={fieldErrorText('passport')}>
                {(msg) => (
                  <span class="desk-error" role="alert" data-field-error="passport">
                    {msg()}
                  </span>
                )}
              </Show>
            </label>
          </div>

          <label class="desk-field">
            <span>{t('notes')}</span>
            <textarea
              name="notes"
              maxLength={500}
              value={notes()}
              onInput={(e) => {
                setNotes(e.currentTarget.value);
                clearField('notes');
              }}
            />
            <Show when={fieldErrorText('notes')}>
              {(msg) => (
                <span class="desk-error" role="alert" data-field-error="notes">
                  {msg()}
                </span>
              )}
            </Show>
          </label>

          <Show when={error()}>
            {(msg) => (
              <p class="desk-error" role="alert">
                {msg()}
              </p>
            )}
          </Show>

          <Show when={saved()}>
            {(tx) => (
              <div class="desk-field" role="status" data-testid="backdate-saved">
                <p>{t('savedTitle')}</p>
                <p>
                  {t('documentRef')} <strong data-testid="document-ref">{tx().documentRef ?? tx().id}</strong>
                </p>
                <a href={`/desk/transactions/${tx().id}`} data-testid="receipt-link">
                  {t('receiptLink')}
                </a>
              </div>
            )}
          </Show>

          <div class="desk-actions">
            <button type="submit" disabled={submitting()}>
              {submitting() ? t('submitting') : t('submit')}
            </button>
          </div>
        </form>

        <aside class="backdated-summary card">
          <div class="desk-rate-line" data-testid="computed-thb">
            <span>{t('computedThb')}:</span>
            <strong>{computedThb() ?? '—'} THB</strong>
            <Show when={settleThb()}>
              {(s) => (
                <span>
                  ({t('settleThb')}: <strong data-testid="computed-settle-thb">{s()} THB</strong>)
                </span>
              )}
            </Show>
          </div>
        </aside>
      </div>
    </div>
  );
}
