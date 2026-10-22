// CA-9b #1372 (design `docs/design/blueprint/cash-shift-transfer.md` §4.7, §9 CA-9b): the limit per currency above
// which a cash transfer needs approval. Reads `cash_transfer_policy` once on open and once after each save; saves
// through `transferPolicySave`. Limits stay the server's strings, or what was typed.
import { createEffect, createSignal, For, Show, untrack, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type {
  CashTransferPolicyData,
  TransferPolicySaveInput,
} from '../../../shared/api/generated/cash_transfer_policy';
import { screenT } from '../../../shared/i18n';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { Card, CardBody, CardHead } from '../../../shared/ui/Card';
import type { AdminMenuProps } from '../menus';
import { toast } from '../parts/toast';
import * as table from './i18n';
import './cash-policy.css';

const t = screenT('admin.cash-policy', table);

/** The refusals this screen words itself; any other shows through wordedError. */
const OWN_TEXT: Record<string, Parameters<typeof t>[0]> = {
  roleNotAllowedHere: 'error.roleNotAllowedHere',
  idempotencyConflict: 'error.idempotencyConflict',
};

const OWN_FIELD_TEXT: Record<string, Parameters<typeof t>[0]> = {
  currency: 'error.currency',
  limits: 'error.limits',
};

function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
}

function isFieldRefusal(err: unknown): boolean {
  if (isServerFailure(err)) return false;
  if (err instanceof ApiError) {
    return (
      err.status === 400 &&
      err.code === 'validationFailed' &&
      typeof err.field === 'string' &&
      err.field in OWN_FIELD_TEXT
    );
  }
  if (err && typeof err === 'object') {
    const e = err as { status?: unknown; code?: unknown; field?: unknown };
    return (
      e.status === 400 &&
      e.code === 'validationFailed' &&
      typeof e.field === 'string' &&
      e.field in OWN_FIELD_TEXT
    );
  }
  return false;
}

function policyError(err: unknown): string {
  if (isServerFailure(err)) {
    return wordedError(err);
  }
  if (isFieldRefusal(err)) {
    const field = (err as { field: string }).field;
    return t(OWN_FIELD_TEXT[field]!);
  }
  if (err instanceof ApiError && err.code in OWN_TEXT) {
    return t(OWN_TEXT[err.code]!);
  }
  if (err && typeof err === 'object' && 'code' in err && typeof (err as { code: unknown }).code === 'string') {
    const code = (err as { code: string }).code;
    if (code in OWN_TEXT) {
      return t(OWN_TEXT[code]!);
    }
  }
  return wordedError(err);
}

export function CashPolicy(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();
  const saveWrite = keyedWrite<TransferPolicySaveInput, CashTransferPolicyData>(
    api,
    'PUT',
    '/api/cash-transfer-policy',
  );

  const [policy, setPolicy] = createSignal<CashTransferPolicyData | null>(null);
  const [known, setKnown] = createSignal(false);
  const [register, setRegister] = createSignal<string[]>([]);
  const [typed, setTyped] = createSignal<Record<string, string>>({});
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const [fieldRefusal, setFieldRefusal] = createSignal<string | null>(null);
  const [registerError, setRegisterError] = createSignal<string | null>(null);

  /** The register the cash screen reads, plus any currency the current version names. */
  const currencies = () => {
    const out = [...register()];
    for (const cur of Object.keys(policy()?.limits ?? {})) if (!out.includes(cur)) out.push(cur);
    return out;
  };

  let readPending: Promise<void> | undefined;
  function readPolicy() {
    readPending = (async () => {
      try {
        const answer = await api.view<CashTransferPolicyData | null>('cash_transfer_policy');
        const data = answer.data ?? null;
        setPolicy(data);
        setTyped({ ...(data?.limits ?? {}) });
        setKnown(true);
        setRefusal(null);
      } catch (err: unknown) {
        setKnown(false);
        setRefusal(policyError(err));
      }
    })();
    return readPending;
  }

  let registerSeq = 0;
  async function readRegister(branch: string) {
    const mine = ++registerSeq;
    try {
      const answer = await api.view<AdminCashData>('admin_cash', { branch });
      if (mine !== registerSeq) return;
      if (answer.data) setRegister(answer.data.currencies);
      setRegisterError(null);
    } catch (err: unknown) {
      if (mine !== registerSeq) return;
      setRegister([]);
      setRegisterError(policyError(err));
    }
  }

  let lastBranch = '';
  createEffect(() => {
    if (!props.shown()) return;
    const branch = props.branch();
    untrack(() => {
      if (lastBranch === branch) return;
      const isSwitch = lastBranch !== '';
      lastBranch = branch;
      if (!isSwitch) {
        void readPolicy();
      }
      void readRegister(branch);
    });
  });

  async function save(e: Event) {
    e.preventDefault();
    if (!known() && !refusal()) await readPending;
    if (!known() || busy()) return;
    const limits: Record<string, string> = {};
    const now = typed();
    for (const cur of currencies()) {
      const value = now[cur] ?? '';
      if (value.trim() !== '') limits[cur] = value;
    }
    const body: TransferPolicySaveInput = { limits };
    setBusy(true);
    setRefusal(null);
    setFieldRefusal(null);
    try {
      await saveWrite.send(body);
      toast(t('saved'));
      await readPolicy();
    } catch (err: unknown) {
      const message = policyError(err);
      if (isFieldRefusal(err)) {
        setFieldRefusal(message);
      } else {
        setRefusal(message);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="admin-cash-policy-screen" data-section="cash-policy">
      <div class="stock-banner">
        <Show
          when={known()}
          fallback={
            <Show when={refusal()}>
              <strong>{t('unreadable')}</strong>
              <button type="button" onClick={() => void readPolicy()}>
                {t('retry')}
              </button>
            </Show>
          }
        >
          <Show when={policy()} fallback={<strong>{t('noLimit')}</strong>}>
            {(p) => (
              <strong>{t('current', { version: p().version, setBy: p().setBy, when: when(p().createdAt) })}</strong>
            )}
          </Show>
        </Show>
        <p>{t('emptyNote')}</p>
      </div>

      <Show when={refusal()}>
        <p role="alert" class="error-message">
          {refusal()}
        </p>
      </Show>

      <Show when={registerError()}>
        <p role="alert" class="error-message">
          {registerError()}
        </p>
      </Show>

      <Card>
        <CardHead eyebrow={t('eyebrow')} title={t('title')} />
        <CardBody>
          <Show when={policy()?.valid === false}>
            <div role="status">
              <p>{t('invalidPolicyNote')}</p>
              <ul>
                <For each={policy()?.invalidEntries ?? []}>
                  {(entry) => (
                    <li>
                      <strong>{entry.currency}</strong>: <code>{entry.raw}</code> ({entry.reason})
                    </li>
                  )}
                </For>
              </ul>
            </div>
          </Show>
          <form onSubmit={save}>
            <Show when={fieldRefusal()}>
              <p role="alert" class="error-message">
                {fieldRefusal()}
              </p>
            </Show>
            <div class="table-scroll">
              <table class="cash-policy-table">
                <thead>
                  <tr>
                    <th>{t('headerCurrency')}</th>
                    <th class="num">{t('headerLimit')}</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={currencies()}>
                    {(cur) => (
                      <tr>
                        <td>
                          <label for={`cash-policy-${cur}`}>
                            <strong>{cur}</strong>
                          </label>
                        </td>
                        <td class="num">
                          <input
                            id={`cash-policy-${cur}`}
                            type="text"
                            inputMode="decimal"
                            data-currency={cur}
                            value={typed()[cur] ?? ''}
                            onInput={(e) => {
                              const value = e.currentTarget.value;
                              setTyped((prev) => ({ ...prev, [cur]: value }));
                            }}
                          />
                        </td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
            <div class="cash-policy-actions">
              <button
                type="submit"
                class="primary"
                data-action="save"
                disabled={(!known() && Boolean(refusal())) || busy()}
                title={!known() && refusal() ? t('unreadable') : undefined}
              >
                {t('save')}
              </button>
            </div>
          </form>
        </CardBody>
      </Card>
    </div>
  );
}
