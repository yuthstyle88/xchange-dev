// XC Counter CA-2 (docs/design/xc-counter-app.md §4, §5.4, §8.4): Admin → สาขา → เคาน์เตอร์, the branch's paired
// counters. Skeleton by claude/bbe9ce8b; the body is CA-2's. BranchesScreen mounts it only for a caller whose
// `me.permissions` holds `branchSettingsSave`, the API's gate (`Permission::BranchSettingsSave`).
//
// Shape (pinned by BranchCounters.test.tsx):
// - one `<section data-counters-panel>` with the heading `countersHeading`;
// - one `[data-counter-row="<agentId>"]` per counter: its last-seen line, a `<label>` holding a text input for the name
//   (saved on `change`), and one `<label>` per setting holding a `<select>` with the options `''` (not set), `'true'`
//   (`counterOn`) and `'false'` (`counterOff`), saved on `change` with the full row;
// - one `<label data-cash-from-browser>` with the same `<select>` for the branch setting.
// Wire: ./counters-api.ts, writes through `keyedWrite`. A refusal is `wordedError` in a `role="alert"`; `staleVersion`
// also re-reads the view, as AlertSettings does. Texts: the `counter*` and `cashFromBrowser*` keys in ./i18n.ts.
import { createEffect, createSignal, For, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import {
  BRANCH_CASH_FROM_BROWSER_SAVE,
  BRANCH_COUNTER_SAVE,
  BRANCH_COUNTERS_VIEW,
  type BranchCashFromBrowserSaveAnswer,
  type BranchCashFromBrowserSaveInput,
  type BranchCounterRow,
  type BranchCounterSaveAnswer,
  type BranchCounterSaveInput,
  type BranchCountersData,
  type CounterSetting,
} from './counters-api';
import * as table from './i18n';

const t = screenT('admin.branches', table);

export interface BranchCountersProps {
  /** The selected branch's id: what `?branch=` and both commands take. */
  branchId: string;
  /** Injected in tests; defaults to a new `ApiClient()`. */
  api?: ApiClient;
}

function toSelectValue(value: CounterSetting): '' | 'true' | 'false' {
  if (value === true) return 'true';
  if (value === false) return 'false';
  return '';
}

function fromSelectValue(value: string): CounterSetting {
  if (value === 'true') return true;
  if (value === 'false') return false;
  return null;
}

interface SettingSelectProps {
  label: string;
  defaultLabel: string;
  value: CounterSetting;
  disabled?: boolean;
  onChange: (next: CounterSetting) => Promise<void>;
  cashFromBrowser?: boolean;
}

function SettingSelect(props: SettingSelectProps): JSX.Element {
  return (
    <label
      class="branches-label"
      data-cash-from-browser={props.cashFromBrowser ? '' : undefined}
    >
      {props.label}
      <select
        class="branches-select"
        disabled={props.disabled}
        value={toSelectValue(props.value)}
        onChange={(e) => {
          const el = e.currentTarget;
          // A refused save leaves the signal as it was, so nothing else puts the saved value back on screen.
          void props.onChange(fromSelectValue(el.value)).finally(() => {
            el.value = toSelectValue(props.value);
          });
        }}
      >
        <option value="" selected={props.value === null}>
          {props.defaultLabel}
        </option>
        <option value="true" selected={props.value === true}>
          {t('counterOn')}
        </option>
        <option value="false" selected={props.value === false}>
          {t('counterOff')}
        </option>
      </select>
    </label>
  );
}

export function BranchCounters(props: BranchCountersProps): JSX.Element {
  const api = props.api ?? new ApiClient();
  const counterWrite = keyedWrite<BranchCounterSaveInput, BranchCounterSaveAnswer>(
    api,
    'POST',
    BRANCH_COUNTER_SAVE,
  );
  const cashFromBrowserWrite = keyedWrite<
    BranchCashFromBrowserSaveInput,
    BranchCashFromBrowserSaveAnswer
  >(api, 'POST', BRANCH_CASH_FROM_BROWSER_SAVE);

  const [counters, setCounters] = createSignal<BranchCounterRow[]>([]);
  const [cashFromBrowser, setCashFromBrowser] = createSignal<CounterSetting>(null);
  const [cashFromBrowserVersion, setCashFromBrowserVersion] = createSignal<number>(0);
  const [error, setError] = createSignal<string | null>(null);
  const [firstLoaded, setFirstLoaded] = createSignal<boolean>(false);
  const [savingBranch, setSavingBranch] = createSignal<boolean>(false);
  const [savingCounters, setSavingCounters] = createSignal<Record<string, boolean>>({});

  let branchSeq = 0;

  const loadView = async (branchId: string, seq = branchSeq) => {
    if (!branchId) return;
    try {
      const res = await api.view<BranchCountersData>(BRANCH_COUNTERS_VIEW, {
        branch: branchId,
      });
      if (seq !== branchSeq || branchId !== props.branchId) return;
      if (res.data) {
        setCounters(res.data.counters);
        setCashFromBrowser(res.data.cashFromBrowser);
        setCashFromBrowserVersion(res.data.cashFromBrowserVersion);
      }
      setFirstLoaded(true);
    } catch (err: unknown) {
      if (seq !== branchSeq || branchId !== props.branchId) return;
      setError(wordedError(err));
    }
  };

  createEffect(() => {
    const branchId = props.branchId;
    const seq = ++branchSeq;
    counterWrite.reset();
    cashFromBrowserWrite.reset();
    setError(null);
    setFirstLoaded(false);
    setSavingBranch(false);
    setSavingCounters({});
    setCounters([]);
    setCashFromBrowser(null);
    setCashFromBrowserVersion(0);
    if (branchId) {
      void loadView(branchId, seq);
    }
  });

  const saveCounter = async (
    row: BranchCounterRow,
    patch: Partial<Pick<BranchCounterRow, 'name' | 'cashDrawer' | 'scanner' | 'kioskLock'>>,
  ) => {
    const branchId = props.branchId;
    if (!branchId || savingCounters()[row.agentId]) return;
    const seq = branchSeq;
    setError(null);
    setSavingCounters((prev) => ({ ...prev, [row.agentId]: true }));
    try {
      const res = await counterWrite.send({
        branch: branchId,
        agentId: row.agentId,
        name: patch.name !== undefined ? patch.name : row.name,
        cashDrawer: patch.cashDrawer !== undefined ? patch.cashDrawer : row.cashDrawer,
        scanner: patch.scanner !== undefined ? patch.scanner : row.scanner,
        kioskLock: patch.kioskLock !== undefined ? patch.kioskLock : row.kioskLock,
      });
      if (seq !== branchSeq || branchId !== props.branchId) return;
      setCounters((prev) =>
        prev.map((c) => (c.agentId === row.agentId ? res.counter : c)),
      );
    } catch (err: unknown) {
      if (seq !== branchSeq || branchId !== props.branchId) return;
      setError(wordedError(err));
    } finally {
      if (seq === branchSeq && branchId === props.branchId) {
        setSavingCounters((prev) => ({ ...prev, [row.agentId]: false }));
      }
    }
  };

  const saveCashFromBrowser = async (value: CounterSetting) => {
    const branchId = props.branchId;
    if (!branchId || savingBranch()) return;
    const seq = branchSeq;
    setError(null);
    setSavingBranch(true);
    try {
      const res = await cashFromBrowserWrite.send({
        branch: branchId,
        expectedVersion: cashFromBrowserVersion(),
        value,
      });
      if (seq !== branchSeq || branchId !== props.branchId) return;
      setCashFromBrowser(res.value);
      setCashFromBrowserVersion(res.version);
    } catch (err: unknown) {
      if (seq !== branchSeq || branchId !== props.branchId) return;
      setError(wordedError(err));
      if (err instanceof ApiError && err.code === 'staleVersion') {
        await loadView(branchId, seq);
      }
    } finally {
      if (seq === branchSeq && branchId === props.branchId) {
        setSavingBranch(false);
      }
    }
  };

  return (
    <section class="branches-card branches-counters" data-counters-panel>
      <h2 class="branches-heading">{t('countersHeading')}</h2>

      <Show when={error()}>
        <div role="alert" class="branches-form-error">
          {error()}
        </div>
      </Show>

      <div class="branches-field">
        <SettingSelect
          label={t('cashFromBrowser')}
          defaultLabel={t('counterDefaultOn')}
          value={cashFromBrowser()}
          disabled={savingBranch()}
          onChange={saveCashFromBrowser}
          cashFromBrowser
        />
      </div>

      <Show
        when={counters().length > 0}
        fallback={
          <Show when={firstLoaded()}>
            <div class="branches-empty">{t('countersEmpty')}</div>
          </Show>
        }
      >
        <div class="branches-counters-list">
          <For each={counters()}>
            {(row) => (
              <div class="branches-counters-row" data-counter-row={row.agentId}>
                <div class="branches-counters-meta">
                  {t('counterLastSeen')}: {row.lastSeenAt ?? t('counterNeverSeen')}
                </div>
                <label class="branches-label">
                  {t('counterName')}
                  <input
                    type="text"
                    class="branches-input"
                    disabled={!!savingCounters()[row.agentId]}
                    value={row.name}
                    onChange={(e) => {
                      const el = e.currentTarget;
                      void saveCounter(row, { name: el.value }).finally(() => {
                        el.value = row.name;
                      });
                    }}
                  />
                </label>
                <SettingSelect
                  label={t('counterCashDrawer')}
                  defaultLabel={t('counterDefaultOff')}
                  value={row.cashDrawer}
                  disabled={!!savingCounters()[row.agentId]}
                  onChange={(next) => saveCounter(row, { cashDrawer: next })}
                />
                <SettingSelect
                  label={t('counterScanner')}
                  defaultLabel={t('counterDefaultOff')}
                  value={row.scanner}
                  disabled={!!savingCounters()[row.agentId]}
                  onChange={(next) => saveCounter(row, { scanner: next })}
                />
                <SettingSelect
                  label={t('counterKioskLock')}
                  defaultLabel={t('counterDefaultOn')}
                  value={row.kioskLock}
                  disabled={!!savingCounters()[row.agentId]}
                  onChange={(next) => saveCounter(row, { kioskLock: next })}
                />
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}
