// FE-admin (#3324), survey row A22: a branch's count-difference limit (`cash_limits` view,
// `PUT /api/cash-limits`; ledger-inventory.md §3.3).
import { createEffect, createSignal, Show, untrack, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { CashLimitsData } from '../../../shared/api/generated/cash_limits';
import type { CashLimitsSaveInput } from '../../../shared/api/generated/cash_limits_save';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { locale, screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { Card, CardBody, CardHead } from '../../../shared/ui/Card';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';

const t = screenT('admin.cash-limits', table);

const OWN_TEXT: Record<string, Parameters<typeof t>[0]> = {
  roleNotAllowedHere: 'error.roleNotAllowedHere',
  idempotencyConflict: 'error.idempotencyConflict',
};

function when(iso: string): string {
  return new Date(iso).toLocaleString(locale(), { timeZone: 'Asia/Bangkok' });
}

function isLimitFieldRefusal(err: unknown): boolean {
  if (isServerFailure(err)) return false;
  if (err instanceof ApiError) {
    return (
      err.status === 400 &&
      (err.code === 'invalidRequest' || err.code === 'validationFailed') &&
      err.field === 'countDifferenceLimitThb'
    );
  }
  if (err && typeof err === 'object') {
    const e = err as { status?: unknown; code?: unknown; field?: unknown };
    return (
      e.status === 400 &&
      (e.code === 'invalidRequest' || e.code === 'validationFailed') &&
      e.field === 'countDifferenceLimitThb'
    );
  }
  return false;
}

function limitsError(err: unknown): string {
  if (isServerFailure(err)) return wordedError(err);
  if (isLimitFieldRefusal(err)) return t('error.countDifferenceLimitThb');
  if (err instanceof ApiError && err.code in OWN_TEXT) return t(OWN_TEXT[err.code]!);
  if (err && typeof err === 'object' && 'code' in err && typeof (err as { code: unknown }).code === 'string') {
    const code = (err as { code: string }).code;
    if (code in OWN_TEXT) return t(OWN_TEXT[code]!);
  }
  return wordedError(err);
}

export function CashLimits(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<CashLimitsSaveInput & { branch: string }, CashLimitsData>(
    api,
    'PUT',
    '/api/cash-limits',
  );

  const [current, setCurrent] = createSignal<CashLimitsData | null>(null);
  const [typed, setTyped] = createSignal('');
  const [noLimit, setNoLimit] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const [fieldRefusal, setFieldRefusal] = createSignal<string | null>(null);

  function applyData(data: CashLimitsData | null) {
    setCurrent(data);
    const limit = data?.countDifferenceLimitThb ?? null;
    if (limit !== null) {
      setTyped(limit);
      setNoLimit(false);
    } else {
      setTyped('');
      setNoLimit(true);
    }
  }

  let readSeq = 0;
  async function readLimits(branch: string) {
    const mine = ++readSeq;
    try {
      const answer = await api.view<CashLimitsData | null>('cash_limits', { branch });
      if (mine !== readSeq) return;
      applyData(answer.data ?? null);
      setRefusal(null);
      setFieldRefusal(null);
    } catch (err: unknown) {
      if (mine !== readSeq) return;
      setRefusal(limitsError(err));
    }
  }

  let lastBranch = '';
  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    untrack(() => {
      if (lastBranch === branch) return;
      lastBranch = branch;
      void readLimits(branch);
    });
  });

  async function save(e: Event) {
    e.preventDefault();
    if (busy()) return;
    const useNoLimit = noLimit();
    const raw = typed();
    if (!useNoLimit && raw.trim() === '') {
      setFieldRefusal(t('error.countDifferenceLimitThb'));
      return;
    }
    const body: CashLimitsSaveInput & { branch: string } = {
      branch: props.branch(),
      countDifferenceLimitThb: useNoLimit ? null : raw,
    };
    setBusy(true);
    setRefusal(null);
    setFieldRefusal(null);
    try {
      const saved = await saveWrite.send(body);
      applyData(saved);
      toast(t('saved'));
    } catch (err: unknown) {
      const message = limitsError(err);
      if (isLimitFieldRefusal(err)) {
        setFieldRefusal(message);
      } else {
        setRefusal(message);
      }
    } finally {
      setBusy(false);
    }
  }

  const shownLimit = () => {
    const c = current();
    return c && c.countDifferenceLimitThb !== null ? c : null;
  };

  return (
    <div class="cash-limits" data-section="cash-limits">
      <div class="stock-banner" data-current-limit>
        <Show when={shownLimit()} fallback={<strong>{t('noLimit')}</strong>}>
          {(c) => (
            <strong>
              {t('current', {
                limit: c().countDifferenceLimitThb ?? '',
                updatedBy: c().updatedBy ?? '',
                updatedAt: c().updatedAt ? when(c().updatedAt!) : '',
              })}
            </strong>
          )}
        </Show>
      </div>

      <Show when={refusal()}>
        <p role="alert" class="error-message" data-form-error>
          {refusal()}
        </p>
      </Show>

      <Card>
        <CardHead eyebrow={t('eyebrow')} title={t('title')} />
        <CardBody>
          <form onSubmit={save}>
            <div class="field-group">
              <label for="cash-limits-amount">{t('limitLabel')}</label>
              <input
                id="cash-limits-amount"
                type="text"
                inputMode="decimal"
                name="countDifferenceLimitThb"
                data-field="countDifferenceLimitThb"
                value={typed()}
                onInput={(e) => {
                  setTyped(e.currentTarget.value);
                  setNoLimit(false);
                }}
              />
              <Show when={fieldRefusal()}>
                <p role="alert" class="error-message" data-field-error="countDifferenceLimitThb">
                  {fieldRefusal()}
                </p>
              </Show>
            </div>

            <div class="field-group">
              <label>
                <input
                  type="checkbox"
                  name="noLimit"
                  data-action="no-limit"
                  checked={noLimit()}
                  onChange={(e) => {
                    setNoLimit(e.currentTarget.checked);
                  }}
                />
                {t('noLimitChoice')}
              </label>
            </div>

            <div class="cash-limits-actions">
              <button type="submit" class="primary" data-action="save" disabled={busy()}>
                {t('save')}
              </button>
            </div>
          </form>
        </CardBody>
      </Card>
    </div>
  );
}
