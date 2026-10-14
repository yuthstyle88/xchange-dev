// CA-9b #1372 (design `docs/design/blueprint/cash-shift-transfer.md` §4.7, §9 CA-9b): the limit per currency above
// which a cash transfer needs approval. Reads `cash_transfer_policy` once on open and once after each save; saves
// through `transferPolicySave`. Limits stay the server's strings, or what was typed.
import { createEffect, createSignal, For, Show, untrack, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { AdminCashData } from '../../../shared/api/generated/admin_cash';
import type {
  CashTransferPolicyData,
  TransferPolicySaveInput,
} from '../../../shared/api/generated/cash_transfer_policy';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
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

function when(iso: string): string {
  return new Date(iso).toLocaleString('th-TH').replace(/\s+/g, ' ').trim();
}

function policyError(err: unknown): string {
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

  const [policy, setPolicy] = createSignal<CashTransferPolicyData | null>(null);
  const [register, setRegister] = createSignal<string[]>([]);
  const [typed, setTyped] = createSignal<Record<string, string>>({});
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);

  /** The register the cash screen reads, plus any currency the current version names. */
  const currencies = () => {
    const out = [...register()];
    for (const cur of Object.keys(policy()?.limits ?? {})) if (!out.includes(cur)) out.push(cur);
    return out;
  };

  async function readPolicy() {
    try {
      const answer = await api.view<CashTransferPolicyData | null>('cash_transfer_policy');
      const data = answer.data ?? null;
      setPolicy(data);
      setTyped({ ...(data?.limits ?? {}) });
    } catch (err: unknown) {
      setRefusal(policyError(err));
    }
  }

  async function readRegister(branch: string) {
    try {
      const answer = await api.view<AdminCashData>('admin_cash', { branch });
      if (answer.data) setRegister(answer.data.currencies);
    } catch (err: unknown) {
      setRegister([]);
      setRefusal(policyError(err));
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

  // One Idempotency-Key per save, kept for a retry of the same body until the server answers.
  let pending: { key: string; body: string } | null = null;

  async function save(e: Event) {
    e.preventDefault();
    if (busy()) return;
    const limits: Record<string, string> = {};
    const now = typed();
    for (const cur of currencies()) {
      const value = now[cur] ?? '';
      if (value.trim() !== '') limits[cur] = value;
    }
    const body: TransferPolicySaveInput = { limits };
    const sent = JSON.stringify(body);
    if (!pending || pending.body !== sent) pending = { key: crypto.randomUUID(), body: sent };
    const key = pending.key;
    setBusy(true);
    setRefusal(null);
    try {
      await api.command<TransferPolicySaveInput, CashTransferPolicyData>('PUT', '/api/cash-transfer-policy', body, key);
      pending = null;
      toast(t('saved'));
      await readPolicy();
    } catch (err: unknown) {
      // a refusal is the server's answer: the next save is a new request; anything else keeps the key
      if (err instanceof ApiError && err.status < 500) pending = null;
      setRefusal(policyError(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="admin-cash-policy-screen" data-section="cash-policy">
      <div class="stock-banner">
        <Show when={policy()} fallback={<strong>{t('noLimit')}</strong>}>
          {(p) => (
            <strong>{t('current', { version: p().version, setBy: p().setBy, when: when(p().createdAt) })}</strong>
          )}
        </Show>
        <p>{t('emptyNote')}</p>
      </div>

      <Show when={refusal()}>
        <p role="alert" class="error-message">
          {refusal()}
        </p>
      </Show>

      <Card>
        <CardHead eyebrow={t('eyebrow')} title={t('title')} />
        <CardBody>
          <form onSubmit={save}>
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
