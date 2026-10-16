import { createEffect, createMemo, createSignal, For, onMount, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { PublishedRatesData } from '../../../shared/api/generated/published_rates';
import type {
  AttendanceClockInput,
  AttendanceClockOutput,
  DayStartServiceInput,
  DayStartServiceOutput,
  DayCutoffInput,
  DayCutoffOutput,
  DayTargetInput,
  DayTargetOutput,
} from '../../../shared/api/generated/branch_day';
import type { StaffCtx } from '../cards';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { ConfirmDialog } from '../parts/ConfirmDialog';
import { th, en } from './i18n';
import './floor.css';

export interface FloorCardProps {
  ctx: StaffCtx;
}


const THB = 'THB';

function getCurrencies(
  rates: PublishedRatesData | null,
  targetAmounts: Record<string, string> | undefined,
  safe: Record<string, string> | null | undefined,
): string[] {
  const set = new Set<string>();
  if (rates?.lines) {
    for (const line of rates.lines) {
      if (line.currency) {
        set.add(line.currency);
      }
    }
  }
  if (targetAmounts) {
    for (const k of Object.keys(targetAmounts)) {
      set.add(k);
    }
  }
  if (safe) {
    for (const k of Object.keys(safe)) {
      set.add(k);
    }
  }
  set.delete(THB);
  const sorted = Array.from(set).sort((a, b) => a.localeCompare(b));
  return [THB, ...sorted];
}

export function FloorCard(props: FloorCardProps): JSX.Element {
  const t = screenT('staff.floor', { th, en });
  const api = new ApiClient();

  const clockCmd = command<AttendanceClockInput, AttendanceClockOutput>(
    api,
    'POST',
    '/api/attendance',
  );
  const startCmd = command<DayStartServiceInput, DayStartServiceOutput>(
    api,
    'POST',
    '/api/branch-day/start',
  );
  const cutoffCmd = command<DayCutoffInput, DayCutoffOutput>(
    api,
    'POST',
    '/api/branch-day/cutoff',
  );
  const targetCmd = command<DayTargetInput, DayTargetOutput>(
    api,
    'POST',
    '/api/branch-day/target',
  );

  const [notice, setNotice] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [showConfirm, setShowConfirm] = createSignal<boolean>(false);
  const [ratesData, setRatesData] = createSignal<PublishedRatesData | null>(null);
  const [busy, setBusy] = createSignal<boolean>(false);

  const [targetAmounts, setTargetAmounts] = createSignal<Record<string, string>>({});
  // Currencies the user typed since the last server answer and has not saved
  // yet; every other input follows the server's target on each re-read (#2635).
  const typedCurrencies = new Set<string>();

  createEffect(() => {
    const amounts = props.ctx.floor?.target?.amounts ?? {};
    setTargetAmounts((prev) => {
      const next: Record<string, string> = { ...amounts };
      for (const cur of typedCurrencies) {
        if (prev[cur] !== undefined) next[cur] = prev[cur];
      }
      return next;
    });
  });

  const loadRates = async () => {
    if (can(props.ctx.me, 'BranchDayOperate') || can(props.ctx.me, 'CashTargetSet')) {
      try {
        const res = await api.view<PublishedRatesData>('staff_published_rates', {
          branch: props.ctx.me.branchId,
        });
        if (res.data) {
          setRatesData(res.data);
        }
      } catch (e: unknown) {
        setError(wordedError(e));
      }
    }
  };

  onMount(() => {
    void loadRates();
  });

  const currencies = createMemo(() =>
    getCurrencies(
      ratesData(),
      props.ctx.floor?.target?.amounts,
      props.ctx.floor?.safe,
    )
  );

  const handleClock = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      setError(null);
      const direction = props.ctx.floor?.myClockIn ? 'OUT' : 'IN';
      await clockCmd.send({
        branch: props.ctx.me.branchId,
        direction,
      });
      setNotice(t('saved'));
      await props.ctx.refresh();
    } catch (e: unknown) {
      setNotice(null);
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleStartService = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      setError(null);
      let currentRates = ratesData();
      if (!currentRates?.publication?.publishedAt) {
        try {
          const res = await api.view<PublishedRatesData>('staff_published_rates', {
            branch: props.ctx.me.branchId,
          });
          if (res.data) {
            currentRates = res.data;
            setRatesData(res.data);
          }
        } catch (e: unknown) {
          setError(wordedError(e));
          return;
        }
      }
      if (!currentRates?.publication?.publishedAt) {
        setError(wordedError(new ApiError({ code: 'serverError', message: '', status: 500 })));
        return;
      }
      await startCmd.send({
        branch: props.ctx.me.branchId,
        equipmentReady: true,
        ratesVerified: true,
        rateVersion: currentRates.publication.publishedAt,
      });
      setNotice(t('saved'));
      await props.ctx.refresh();
    } catch (e: unknown) {
      setNotice(null);
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleCutoff = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      setError(null);
      await cutoffCmd.send({
        branch: props.ctx.me.branchId,
      });
      setNotice(t('saved'));
      await props.ctx.refresh();
    } catch (e: unknown) {
      setNotice(null);
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  const handleTargetSubmit = async (e: Event) => {
    e.preventDefault();
    if (busy()) return;
    setBusy(true);
    try {
      setError(null);
      const amounts: Record<string, string> = {};
      const curs = currencies();
      const currentAmounts = targetAmounts();
      for (const cur of curs) {
        const val = currentAmounts[cur];
        if (val !== undefined && val.trim() !== '') {
          amounts[cur] = val.trim();
        }
      }
      await targetCmd.send({
        branch: props.ctx.me.branchId,
        amounts,
      });
      typedCurrencies.clear();
      setNotice(t('saved'));
      await props.ctx.refresh();
    } catch (e: unknown) {
      setNotice(null);
      setError(wordedError(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div class="xc-floor-card">
      <Show when={notice()}>
        {(msg) => <div data-testid="floor-notice" class="xc-floor-notice">{msg()}</div>}
      </Show>
      <Show when={error()}>
        {(err) => <div data-testid="floor-error" class="xc-floor-error">{err()}</div>}
      </Show>

      <div data-testid="floor-phase" class="xc-floor-phase">
        {props.ctx.floor?.phase ? t(`phase.${props.ctx.floor.phase}` as any) : ''}
      </div>

      <div class="xc-floor-actions">
        <button
          type="button"
          data-testid="floor-clock"
          class="xc-floor-clock-btn"
          disabled={busy()}
          onClick={handleClock}
        >
          {props.ctx.floor?.myClockIn ? t('clockOut') : t('clockIn')}
        </button>

        <Show when={can(props.ctx.me, 'BranchDayOperate')}>
          <Show when={props.ctx.floor?.phase === 'READY'}>
            <button
              type="button"
              data-testid="floor-start"
              class="xc-floor-start-btn"
              disabled={busy()}
              onClick={() => {
                if (!busy()) setShowConfirm(true);
              }}
            >
              {t('startService')}
            </button>
          </Show>
          <Show when={props.ctx.floor?.phase === 'OPEN'}>
            <button
              type="button"
              data-testid="floor-cutoff"
              class="xc-floor-cutoff-btn"
              disabled={busy()}
              onClick={handleCutoff}
            >
              {t('cutoff')}
            </button>
          </Show>
        </Show>
      </div>

      <Show when={can(props.ctx.me, 'CashTargetSet')}>
        <form
          data-testid="floor-target-form"
          class="xc-floor-target-form"
          onSubmit={handleTargetSubmit}
        >
          <For each={currencies()}>
            {(cur) => (
              <div class="xc-floor-target-field">
                <label class="xc-floor-target-label">{cur}</label>
                <input
                  type="text"
                  data-currency={cur}
                  class="xc-floor-target-input"
                  value={targetAmounts()[cur] ?? ''}
                  onInput={(e) => {
                    const val = e.currentTarget.value;
                    typedCurrencies.add(cur);
                    setTargetAmounts((prev) => ({ ...prev, [cur]: val }));
                  }}
                />
              </div>
            )}
          </For>
          <button
            type="submit"
            data-testid="floor-target-save"
            class="xc-floor-target-save-btn"
            disabled={busy()}
          >
            {t('targetSave')}
          </button>
        </form>
      </Show>

      <Show when={props.ctx.floor?.target}>
        {(tg) => (
          <div data-testid="floor-target" class="xc-floor-target">
            <For each={Object.entries(tg().amounts ?? {})}>
              {([cur, amount]) => (
                <span class="xc-floor-target-amount">
                  {cur}: {amount}
                </span>
              )}
            </For>
          </div>
        )}
      </Show>

      <Show when={props.ctx.floor?.safe}>
        {(sf) => (
          <div data-testid="floor-safe" class="xc-floor-safe">
            <For each={Object.entries(sf())}>
              {([cur, amount]) => (
                <span class="xc-floor-safe-amount">
                  {cur}: {amount}
                </span>
              )}
            </For>
          </div>
        )}
      </Show>

      <ConfirmDialog
        open={showConfirm() && !busy()}
        title={t('startService')}
        message={t('startConfirm')}
        onConfirm={() => {
          if (busy()) return;
          setShowConfirm(false);
          void handleStartService();
        }}
        onCancel={() => setShowConfirm(false)}
      />
    </div>
  );
}
