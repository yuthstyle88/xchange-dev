// BT-F3 (kiosk-shared-phone §7.3 T-D2, T-D4, T-D5, T-D10; §7.7 `POST /api/branch-tablets`; §7.8 rows T1, T6–T8 on
// the panel; §7.9): the branch's tablets, on the admin branches screen. Skeleton by the XC FE controller
// claude/e8020026; the body is BT-F3's. BranchesScreen mounts it only for a caller whose `me.permissions` holds
// `SystemSettings` (T-D5, Q-T2), and only for a branch that has a code (the QR needs `?branch=<code>`).
//
// Wire (BT-4, `api/src/presentation/http/branch_tablets.rs`): every call is `POST /api/branch-tablets` through
// `ApiClient.post` (never a raw fetch with POST):
// - `{action: "list"}` ⇒ the tablets of the branches the caller may see, each with `state` `PENDING` · `PAIRED` ·
//   `REVOKED` and `lastSeenAt`. The panel shows only the rows of `branchId`.
// - `{action: "add", payload: {branchId, name}}` ⇒ `{tabletId, code, codeExpiresAt}`.
// - `{action: "newCode", payload: {tabletId}}` ⇒ the same shape (re-pairs; the old token stops working, T7).
// - `{action: "revoke", payload: {tabletId}}` ⇒ `{saved: true}` (T6).
// - A refusal (`OwnerOnly`, out of scope, T8) is shown worded (`wordedError`) as `role="alert"`.
// The code of `add` / `newCode` is shown once, as text plus a `QrCode` of
// `<location.origin>/app/queue?branch=<branchCode>#pair=<CODE>` (T-D10), with `codeExpiresAt`; it is not kept after
// the panel moves on (a reload or another branch shows no code).
// Texts: `admin.branches` `tablet*` keys in ./i18n.ts (already there; BT-F3 adds none).
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { QrCode } from '../../../shared/ui/QrCode';
import * as table from './i18n';

const t = screenT('admin.branches', table);

export type TabletState = 'PENDING' | 'PAIRED' | 'REVOKED';

export interface TabletRow {
  tabletId: string;
  branchId: string;
  name: string;
  state: TabletState;
  lastSeenAt: string | null;
}

interface IssuedCode {
  tabletId: string;
  code: string;
  codeExpiresAt: string;
}

export interface BranchTabletsProps {
  /** The selected branch's id: what `add` and the list filter take. */
  branchId: string;
  /** The selected branch's code: what the pairing QR's `?branch=` takes. */
  branchCode: string;
  /** Injected in tests; defaults to a new `ApiClient()`. */
  api?: ApiClient;
}

function stateLabel(state: TabletState): string {
  switch (state) {
    case 'PENDING':
      return t('tabletStatePending');
    case 'PAIRED':
      return t('tabletStatePaired');
    case 'REVOKED':
      return t('tabletStateRevoked');
  }
}

export function BranchTablets(props: BranchTabletsProps): JSX.Element {
  const api = () => props.api ?? new ApiClient();

  const [rows, setRows] = createSignal<TabletRow[]>([]);
  const [name, setName] = createSignal<string>('');
  const [issued, setIssued] = createSignal<IssuedCode | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);

  let branchSeq = 0;

  const loadList = async (seq = branchSeq) => {
    try {
      const res = await api().post<{ action: 'list' }, { tablets?: TabletRow[] }>(
        '/api/branch-tablets',
        { action: 'list' },
      );
      if (seq !== branchSeq) return;
      setRows(res.tablets ?? []);
    } catch (err: unknown) {
      if (seq !== branchSeq) return;
      setError(wordedError(err));
    }
  };

  createEffect(() => {
    const branchId = props.branchId;
    void props.branchCode;
    const seq = ++branchSeq;
    setIssued(null);
    setError(null);
    setStatus(null);
    setRows([]);
    if (branchId) {
      void loadList(seq);
    }
  });

  const branchRows = () => rows().filter((r) => r.branchId === props.branchId);

  const pairUrl = (code: string) =>
    `${window.location.origin}/app/queue?branch=${encodeURIComponent(props.branchCode)}#pair=${code}`;

  const handleAdd = async (e: Event) => {
    e.preventDefault();
    const trimmed = name().trim();
    if (!trimmed) {
      setIssued(null);
      setStatus(null);
      setError(t('tabletNameRequired'));
      return;
    }
    const seq = branchSeq;
    setError(null);
    setStatus(null);
    setIssued(null);
    try {
      const res = await api().post<
        { action: 'add'; payload: { branchId: string; name: string } },
        IssuedCode
      >('/api/branch-tablets', {
        action: 'add',
        payload: { branchId: props.branchId, name: trimmed },
      });
      if (seq !== branchSeq) return;
      setName('');
      setIssued(res);
      await loadList(seq);
    } catch (err: unknown) {
      if (seq !== branchSeq) return;
      setIssued(null);
      setError(wordedError(err));
    }
  };

  const handleNewCode = async (tabletId: string) => {
    const seq = branchSeq;
    setError(null);
    setStatus(null);
    setIssued(null);
    try {
      const res = await api().post<
        { action: 'newCode'; payload: { tabletId: string } },
        IssuedCode
      >('/api/branch-tablets', {
        action: 'newCode',
        payload: { tabletId },
      });
      if (seq !== branchSeq) return;
      setIssued(res);
    } catch (err: unknown) {
      if (seq !== branchSeq) return;
      setIssued(null);
      setError(wordedError(err));
    }
  };

  const handleRevoke = async (tabletId: string) => {
    const seq = branchSeq;
    setError(null);
    setStatus(null);
    setIssued(null);
    try {
      await api().post<
        { action: 'revoke'; payload: { tabletId: string } },
        { saved: boolean }
      >('/api/branch-tablets', {
        action: 'revoke',
        payload: { tabletId },
      });
      if (seq !== branchSeq) return;
      setStatus(t('tabletRevoked'));
      await loadList(seq);
    } catch (err: unknown) {
      if (seq !== branchSeq) return;
      setIssued(null);
      setError(wordedError(err));
    }
  };

  return (
    <section class="branches-card branches-tablets" data-section="branch-tablets">
      <h2 class="branches-heading">{t('tabletsHeading')}</h2>
      <p class="branches-tablets-intro">{t('tabletsIntro')}</p>

      <Show when={error()}>
        <div role="alert" class="branches-form-error">{error()}</div>
      </Show>
      <Show when={status()}>
        <div role="status" class="branches-form-success">{status()}</div>
      </Show>

      <form class="branches-tablets-add" onSubmit={handleAdd}>
        <div class="branches-field">
          <label class="branches-label" for="tablet-name-input">{t('tabletName')}</label>
          <input
            id="tablet-name-input"
            name="tabletName"
            class="branches-input"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
        </div>
        <button
          type="submit"
          class="branches-btn branches-btn-primary"
          data-action="tablet-add"
        >
          {t('tabletAdd')}
        </button>
      </form>

      <Show when={issued()}>
        {(codeInfo) => (
          <div class="branches-tablets-code" data-section="tablet-code">
            <div>
              <span class="branches-label">{t('tabletCode')}: </span>
              <span class="branches-tablets-code-value">{codeInfo().code}</span>
            </div>
            <p class="branches-tablets-code-meta">{t('tabletCodeOnce')}</p>
            <p class="branches-tablets-code-meta">
              {t('tabletCodeExpires', { time: codeInfo().codeExpiresAt })}
            </p>
            <code class="branches-tablets-code-link">{pairUrl(codeInfo().code)}</code>
            <QrCode value={pairUrl(codeInfo().code)} label={t('tabletQrLabel')} />
          </div>
        )}
      </Show>

      <Show
        when={branchRows().length > 0}
        fallback={<div class="branches-empty">{t('tabletsEmpty')}</div>}
      >
        <div class="branches-tablets-list">
          <For each={branchRows()}>
            {(row) => (
              <div class="branches-tablets-row" data-tablet-id={row.tabletId}>
                <div class="branches-tablets-info">
                  <span class="branches-tablets-name">{row.name}</span>
                  <span class="branches-tablets-meta">
                    {t('tabletState')}: {stateLabel(row.state)} · {t('tabletLastSeen')}:{' '}
                    {row.lastSeenAt ?? t('tabletNeverSeen')}
                  </span>
                </div>
                <Show when={row.state !== 'REVOKED'}>
                  <div class="branches-tablets-actions">
                    <button
                      type="button"
                      class="branches-btn"
                      data-action="tablet-new-code"
                      onClick={() => void handleNewCode(row.tabletId)}
                    >
                      {t('tabletNewCode')}
                    </button>
                    <button
                      type="button"
                      class="branches-btn"
                      data-action="tablet-revoke"
                      onClick={() => void handleRevoke(row.tabletId)}
                    >
                      {t('tabletRevoke')}
                    </button>
                  </div>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}
