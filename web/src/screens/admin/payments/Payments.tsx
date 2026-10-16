// V2-payments (#470): payment settings menu component (§7, §10, design §12.3, §12.6)

import {
  createEffect,
  createMemo,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  untrack,
  type JSX,
} from 'solid-js';
import { ApiClient } from '../../../shared/api/client';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import type {
  AdminPaymentsData,
  PaymentHistoryItem,
  PaymentMethodItem,
} from '../../../shared/api/generated/admin_payments';
import type { AdminMenuProps } from '../menus';
import { refreshTick } from '../parts/refresh';
import { Card, CardBody, CardGrid } from '../../../shared/ui/Card';
import { Field, CheckField } from '../../../shared/ui/Field';
import { en, th } from './i18n';
import { formatDate } from './text';
import './payments.css';

const t = screenT('admin.payments', { th, en });

const CHANNELS = [
  { id: 'Cash', nameKey: 'channelCash' as const, line: 'Cash', icon: '฿' },
  { id: 'Credit Card', nameKey: 'channelCreditCard' as const, line: 'Credit card', icon: '▣' },
  { id: 'Alipay', nameKey: 'channelAlipay' as const, line: 'Alipay', icon: '支' },
  { id: 'PromptPay', nameKey: 'channelPromptPay' as const, line: 'PromptPay', icon: '▦' },
  { id: 'Bank Transfer', nameKey: 'channelBankTransfer' as const, line: 'Bank transfer', icon: '⇄' },
] as const;

const BUY_CHANNELS: readonly string[] = ['Cash', 'Bank Transfer'];

interface BranchInfo {
  id: string;
  name: string;
}

export function Payments(props: AdminMenuProps): JSX.Element {
  const api = new ApiClient();

  const [branches, setBranches] = createSignal<BranchInfo[]>([]);
  const [displayedBranch, setDisplayedBranch] = createSignal<string>('');
  const [branchNotice, setBranchNotice] = createSignal<string | null>(null);

  const [data, setData] = createSignal<AdminPaymentsData | null>(null);
  const [version, setVersion] = createSignal<number | null>(null);
  const [loading, setLoading] = createSignal<boolean>(false);
  const [error, setError] = createSignal<string | null>(null);

  const [edits, setEdits] = createSignal<PaymentMethodItem[] | null>(null);
  const [saving, setSaving] = createSignal<boolean>(false);
  const [feedback, setFeedback] = createSignal<string>('');

  let pendingBranchSwitch: string | null = null;
  let savePromise: Promise<void> | null = null;

  const isDirty = () => edits() !== null;
  const canEdit = () => data()?.canEdit ?? false;

  const branchDisplayName = createMemo(() => {
    const bId = displayedBranch();
    const found = branches().find((b) => b.id === bId);
    return found ? found.name : bId;
  });

  const currentMethods = createMemo<PaymentMethodItem[]>(() => {
    const e = edits();
    if (e) return e;
    const d = data();
    if (d) return d.methods;
    return [];
  });

  const openCount = createMemo(() => {
    const list = currentMethods();
    const open = list.filter((m) => m.enabled).length;
    const total = list.length;
    return t('openCount', { open, total });
  });

  const metaText = createMemo(() => {
    const d = data();
    if (!d || !d.updatedAt) {
      return t('neverEdited');
    }
    return t('editedMeta', {
      date: formatDate(d.updatedAt),
      by: d.updatedBy ?? '',
    });
  });

  const onBeforeUnload = (e: BeforeUnloadEvent) => {
    if (isDirty()) {
      e.preventDefault();
    }
  };

  onMount(() => {
    window.addEventListener('beforeunload', onBeforeUnload);
  });

  onCleanup(() => {
    window.removeEventListener('beforeunload', onBeforeUnload);
  });

  // Load branches for branch name display in heading. `stale` (#2246) tells a superseded read to
  // drop its answer or error.
  const loadBranches = async (stale: () => boolean = () => false) => {
    try {
      const res = await api.view<{ branches: BranchInfo[] }>('admin_branches');
      if (stale()) return;
      if (res.data?.branches) {
        setBranches(res.data.branches);
      }
    } catch (err: unknown) {
      if (stale()) return;
      setBranches([]);
      setError(wordedError(err));
    }
  };

  // #2246: every read takes a number; only the latest read for the displayed branch may land.
  let loadSeq = 0;

  const loadData = async (branch: string, since?: number) => {
    const seq = ++loadSeq;
    const stale = () => seq !== loadSeq || branch !== displayedBranch();
    if (data() && data()!.branchId !== branch) {
      setData(null);
      setVersion(null);
      setEdits(null);
    }
    setLoading(true);
    setError(null);
    try {
      await loadBranches(stale);
      if (stale()) return;
      const res = await api.view<AdminPaymentsData>('admin_payments', {
        branch,
        since,
      });
      if (stale()) return;
      if (res.changed && res.data) {
        setData(res.data);
        setVersion(res.version);
        setEdits(null);
      } else if (!res.changed) {
        // unchanged: keep data, update version
        setVersion(res.version);
      }
    } catch (err: unknown) {
      if (stale()) return;
      if (data() && data()!.branchId !== branch) {
        setData(null);
        setVersion(null);
        setEdits(null);
      }
      setError(wordedError(err));
    } finally {
      if (seq === loadSeq) setLoading(false);
    }
  };

  // Tracking shown & branch switch from shell
  createEffect(() => {
    const isShown = props.shown();
    const targetBranch = props.branch();
    if (!isShown) return;

    untrack(() => {
      const current = displayedBranch();
      if (current === '') {
        setDisplayedBranch(targetBranch);
        void loadData(targetBranch);
        return;
      }

      if (targetBranch === current) return;

      if (saving()) {
        pendingBranchSwitch = targetBranch;
        return;
      }

      if (isDirty()) {
        const confirmed = window.confirm(t('confirmBranch'));
        if (confirmed) {
          setEdits(null);
          setBranchNotice(null);
          // #2489: save feedback describes the previous branch; drop it on switch.
          setFeedback('');
          setDisplayedBranch(targetBranch);
          void loadData(targetBranch);
        } else {
          setBranchNotice(t('branchNotice', { branch: branchDisplayName() }));
        }
      } else {
        setBranchNotice(null);
        setFeedback('');
        setDisplayedBranch(targetBranch);
        void loadData(targetBranch);
      }
    });
  });

  // Refresh tick (design §12.6)
  createEffect(
    on(
      refreshTick,
      (tick) => {
        if (tick > 0 && props.shown() && !untrack(isDirty) && !untrack(saving)) {
          const v = untrack(version);
          const b = untrack(displayedBranch);
          void loadData(b, v !== null ? v : undefined);
        }
      },
      { defer: true },
    ),
  );

  const updateMethod = (id: string, patch: Partial<PaymentMethodItem>) => {
    const base = currentMethods();
    const next = base.map((m) => (m.id === id ? { ...m, ...patch } : { ...m }));

    const d = data();
    if (!d) return;

    let dirty = false;
    for (const m of next) {
      const orig = d.methods.find((o) => o.id === m.id);
      if (
        !orig ||
        orig.enabled !== m.enabled ||
        orig.buy !== m.buy ||
        orig.sell !== m.sell ||
        orig.account !== m.account ||
        orig.instruction !== m.instruction
      ) {
        dirty = true;
        break;
      }
    }

    if (dirty) {
      setEdits(next);
    } else {
      setEdits(null);
    }
  };

  const handleDiscard = () => {
    if (saving()) return;
    setEdits(null);
    setFeedback('');
    if (displayedBranch() !== props.branch()) {
      const nextBranch = props.branch();
      setBranchNotice(null);
      setDisplayedBranch(nextBranch);
      void loadData(nextBranch);
    }
  };

  const handleReload = () => {
    if (saving()) return;
    if (isDirty()) {
      const confirmed = window.confirm(t('confirmReload'));
      if (!confirmed) return;
    }
    setEdits(null);
    // #2489: a reload starts a fresh view; old save feedback no longer applies.
    setFeedback('');
    void loadData(displayedBranch());
  };

  const handleSave = async () => {
    if (saving() || !canEdit()) return;

    setSaving(true);
    setFeedback(t('saving'));

    const branchToSave = displayedBranch();
    const methodsToSave = currentMethods().map((m) => ({
      id: m.id,
      enabled: m.enabled,
      buy: m.buy,
      sell: m.sell,
      account: m.account.trim(),
      instruction: m.instruction.trim(),
    }));

    const currentVersion = data()?.settingsVersion ?? 0;

    const execute = async () => {
      try {
        await api.post('/api/payment-methods', {
          action: 'save',
          payload: {
            branchId: branchToSave,
            version: currentVersion,
            methods: methodsToSave,
          },
        });
        // #2489: the answer only describes branchToSave; never word it under another branch.
        if (displayedBranch() === branchToSave) setFeedback(t('saved'));
        setEdits(null);
        await loadData(branchToSave);

        if (pendingBranchSwitch && pendingBranchSwitch !== displayedBranch()) {
          const next = pendingBranchSwitch;
          pendingBranchSwitch = null;
          setBranchNotice(null);
          setFeedback('');
          setDisplayedBranch(next);
          await loadData(next);
        } else if (displayedBranch() !== props.branch()) {
          const next = props.branch();
          setBranchNotice(null);
          setFeedback('');
          setDisplayedBranch(next);
          await loadData(next);
        }
      } catch (err: unknown) {
        if (displayedBranch() === branchToSave) setFeedback(wordedError(err));
      } finally {
        setSaving(false);
        savePromise = null;
      }
    };

    savePromise = execute();
    await savePromise;
  };

  const computeChangedChannels = (
    before: PaymentMethodItem[],
    after: PaymentMethodItem[],
  ): string => {
    const changed: string[] = [];
    for (const c of CHANNELS) {
      const b = before.find((m) => m.id === c.id);
      const a = after.find((m) => m.id === c.id);
      if (!b || !a) {
        changed.push(t(c.nameKey));
      } else if (
        b.enabled !== a.enabled ||
        b.buy !== a.buy ||
        b.sell !== a.sell ||
        b.account !== a.account ||
        b.instruction !== a.instruction
      ) {
        changed.push(t(c.nameKey));
      }
    }
    if (changed.length === 0) {
      return t('unchanged');
    }
    return changed.join(', ');
  };

  return (
    <div class="xc-payments">
      <Show when={loading() && !data()}>
        <Card class="xc-payment-loading">
          <p role="status">{t('loading')}</p>
        </Card>
      </Show>

      <Show when={error() && !data()}>
        <Card class="xc-payment-loading">
          <div class="error-banner" role="alert">
            <p role="status">{error()}</p>
          </div>
          <button
            type="button"
            class="secondary"
            onClick={() => void loadData(displayedBranch())}
          >
            {t('retry')}
          </button>
        </Card>
      </Show>

      <Show when={error() && data()}>
        <div class="error-banner" role="alert">
          {error()}
        </div>
      </Show>

      <Show when={data()}>
        <div class="xc-payment-header">
          <div>
            <p class="eyebrow">{t('introEyebrow')}</p>
            <h2>
              {t('headingOf')} {branchDisplayName()}
            </h2>
            <p class="xc-payment-intro">{t('intro')}</p>
          </div>
          <span class="pill xc-payment-count-pill">{openCount()}</span>
        </div>

        <div class="xc-payment-meta-bar">
          <span class="xc-payment-meta-text">{metaText()}</span>
          <button
            type="button"
            disabled={saving()}
            onClick={handleReload}
            class="secondary"
          >
            {t('reload')}
          </button>
        </div>

        <Show when={branchNotice()}>
          <div class="xc-branch-notice">{branchNotice()}</div>
        </Show>

        <Show when={!canEdit()}>
          <p class="stock-banner">{t('readOnly')}</p>
        </Show>

        <form
          onSubmit={(e) => {
            e.preventDefault();
            void handleSave();
          }}
        >
          <fieldset disabled={saving() || !canEdit()} class="xc-payment-fieldset">
            <CardGrid columns={2}>
              <For each={CHANNELS}>
                {(c) => {
                  const method = () =>
                    currentMethods().find((m) => m.id === c.id) ?? {
                      id: c.id,
                      enabled: false,
                      buy: false,
                      sell: false,
                      account: '',
                      instruction: '',
                    };

                  return (
                    <div data-payment-id={c.id} style="display: contents">
                      <Card class="xc-payment-channel">
                        <CardBody>
                          <div class="xc-payment-card-head">
                            <span class="xc-payment-icon" aria-hidden="true">
                              {c.icon}
                            </span>
                            <div>
                              <h3>{t(c.nameKey)}</h3>
                              <p class="xc-payment-line">{c.line}</p>
                            </div>
                            <span class="xc-payment-switch">
                              <CheckField
                                label={t('enabled')}
                                checked={method().enabled}
                                onChange={(checked) =>
                                  updateMethod(c.id, { enabled: checked })
                                }
                              />
                            </span>
                          </div>

                          <div class="xc-payment-directions">
                            <CheckField
                              label={t('sell')}
                              hint={t('sellNote')}
                              checked={method().sell}
                              onChange={(checked) =>
                                updateMethod(c.id, { sell: checked })
                              }
                            />

                            <Show
                              when={BUY_CHANNELS.includes(c.id)}
                              fallback={
                                <CheckField
                                  label={t('buy')}
                                  hint={t('receiveOnly')}
                                  checked={false}
                                  disabled={true}
                                  onChange={() => {}}
                                />
                              }
                            >
                              <CheckField
                                label={t('buy')}
                                hint={t('buyNote')}
                                checked={method().buy}
                                onChange={(checked) =>
                                  updateMethod(c.id, { buy: checked })
                                }
                              />
                            </Show>
                          </div>

                          <Field label={t('account')} unit={t('accountUnit')}>
                            <input
                              type="text"
                              maxLength={100}
                              placeholder={
                                c.id === 'Cash' ? t('accountCash') : t('accountOther')
                              }
                              value={method().account}
                              onInput={(e) =>
                                updateMethod(c.id, { account: e.currentTarget.value })
                              }
                            />
                          </Field>

                          <Field label={t('instruction')} unit={t('instructionUnit')}>
                            <textarea
                              maxLength={400}
                              placeholder={t('instructionPlaceholder')}
                              value={method().instruction}
                              onInput={(e) =>
                                updateMethod(c.id, { instruction: e.currentTarget.value })
                              }
                            />
                          </Field>

                          <div class="xc-payment-evidence">
                            {c.id === 'Cash' ? t('evidenceCash') : t('evidenceOther')}
                          </div>
                        </CardBody>
                      </Card>
                    </div>
                  );
                }}
              </For>
            </CardGrid>
          </fieldset>

          <div class="xc-payment-save-bar">
            <div class="xc-payment-save-status">
              <strong class="xc-payment-dirty-state">{isDirty() ? t('dirty') : t('clean')}</strong>
              <span class="xc-payment-save-note">{t('saveNote')}</span>
            </div>

            <div class="xc-payment-save-actions">
              <button
                type="button"
                disabled={!isDirty() || saving()}
                onClick={handleDiscard}
                class="secondary"
              >
                {t('discard')}
              </button>
              <button
                type="submit"
                disabled={!canEdit() || saving()}
                class="primary"
              >
                {t('save')}
              </button>
            </div>
          </div>

          <p role="status" id="payment-feedback" class="error xc-payment-feedback">
            {feedback()}
          </p>

          <p class="footnote">{t('footnote')}</p>
        </form>

        <Show when={data()?.history !== undefined}>
          <details class="xc-payment-history-details">
            <summary>
              {t('historySummary', { count: data()!.history.length })}
            </summary>
            <div class="table-scroll">
              <table class="xc-payment-history-table">
                <thead>
                  <tr>
                    <th>{t('historyHeaderDate')}</th>
                    <th>{t('historyHeaderActor')}</th>
                    <th>{t('historyHeaderChanges')}</th>
                    <th>{t('historyHeaderVersion')}</th>
                  </tr>
                </thead>
                <tbody>
                  <Show
                    when={data()!.history.length > 0}
                    fallback={
                      <tr>
                        <td colspan="4">{t('noHistory')}</td>
                      </tr>
                    }
                  >
                    <For each={data()!.history}>
                      {(h: PaymentHistoryItem) => (
                        <tr>
                          <td>{formatDate(h.createdAt)}</td>
                          <td>{h.actor}</td>
                          <td>{computeChangedChannels(h.beforeMethods, h.afterMethods)}</td>
                          <td>{String(h.version)}</td>
                        </tr>
                      )}
                    </For>
                  </Show>
                </tbody>
              </table>
            </div>
          </details>
        </Show>
      </Show>
    </div>
  );
}
