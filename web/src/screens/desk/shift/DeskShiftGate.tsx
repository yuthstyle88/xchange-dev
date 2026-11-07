import { createEffect, createSignal, Show, type Accessor, type JSX } from 'solid-js';
import type { Me } from '../../../shared/api/generated/common';
import { screenT } from '../../../shared/i18n';
import { sharedApi, sharedSession } from '../../../shared/session/session';
import { ClockButton } from '../../staff/floor/ClockButton';
import { can } from '../../staff/parts/can';
import { CountForm, CountWaitingLine } from '../../staff/shift/CountForm';
import { myWaitingOpenCount } from '../../staff/shift/counts';
import { createWaitingCounts } from '../../staff/shift/waiting-counts';
import { createStaffContext } from '../../staff/staff-context';
import { deskPathFor } from '../desk-path';
import { pageBranch } from '../desk-store';
import * as screenI18n from '../i18n';
import { deskShiftPhase } from './phase';
import './shift-gate.css';

export interface DeskShift {
  open(): boolean;
  Panel(): JSX.Element;
}

export function createDeskShiftGate(opts?: {
  me?: Accessor<Me | undefined>;
  branch?: Accessor<string>;
}): DeskShift {
  const t = screenT('desk', screenI18n);
  const api = sharedApi();
  const me = opts?.me ?? sharedSession().me;
  const branch =
    opts?.branch ??
    (() => pageBranch(deskPathFor(typeof location !== 'undefined' ? location.pathname : ''), me()));
  const signInUrl =
    '/login?next=' +
    encodeURIComponent(typeof location !== 'undefined' ? location.pathname + location.search : '');
  const navigate = (url: string) => {
    window.location.assign(url);
  };

  const staff = createStaffContext({ api, branch, signInUrl, navigate });
  const counts = createWaitingCounts({
    api,
    branch,
    enabled: () => can(me(), 'ShiftOperate'),
  });

  const [busy, setBusy] = createSignal(false);
  const [writeError, setWriteError] = createSignal<string | null>(null);
  const [counting, setCounting] = createSignal(false);

  let started = false;
  createEffect(() => {
    if (!can(me(), 'ShiftOperate')) return;
    if (!started) {
      started = true;
      void staff.refresh();
    }
    if (!counts.loaded()) {
      void counts.reload();
    }
  });

  const openWaiting = () => {
    const sm = staff.me();
    return sm ? myWaitingOpenCount(counts.rows(), sm.userId) : undefined;
  };

  const phase = () =>
    deskShiftPhase({
      sessionMe: me(),
      staffMe: staff.me(),
      staffError: staff.error(),
      floor: staff.floor(),
      countsLoaded: counts.loaded(),
      openWaiting: openWaiting(),
    });

  const open = () => phase() === 'open';
  const adminHref = () => (branch() ? `/?branch=${encodeURIComponent(branch())}` : '/');

  const Panel = (): JSX.Element => (
    <div class="desk-shift-gate" data-desk-shift={phase()}>
      <Show when={writeError()}>
        {(msg) => (
          <p class="desk-error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
      <Show when={phase() === 'loading'}>
        <p class="desk-notice">{t('shift.checking')}</p>
      </Show>
      <Show when={phase() === 'error'}>
        <p class="desk-error" role="alert">
          {staff.error()}
        </p>
        <div class="desk-actions">
          <button type="button" data-action="shift-retry" onClick={() => void staff.refresh()}>
            {t('shift.retry')}
          </button>
        </div>
      </Show>
      <Show when={phase() === 'noAccess'}>
        <p class="desk-notice">
          <a class="desk-shift-admin-link" href={adminHref()}>{t('shift.noShift')}</a>
        </p>
      </Show>
      <Show when={phase() === 'clockIn' && staff.ctx()}>
        {(ctx) => (
          <>
            <p class="desk-notice">{t('shift.clockInFirst')}</p>
            <div class="desk-actions">
              <ClockButton
                ctx={ctx()}
                busy={busy}
                setBusy={setBusy}
                onNotice={() => {}}
                onError={setWriteError}
              />
            </div>
          </>
        )}
      </Show>
      <Show when={phase() === 'waiting' && openWaiting()} keyed>
        {(row) => <CountWaitingLine kind="OPEN" row={row} />}
      </Show>
      <Show when={phase() === 'openCount' && staff.ctx()}>
        {(ctx) => (
          <Show
            when={counting()}
            fallback={
              <>
                <p class="desk-notice">{t('shift.openCountFirst')}</p>
                <div class="desk-actions">
                  <button
                    type="button"
                    data-testid="shift-open-count"
                    onClick={() => setCounting(true)}
                  >
                    {t('shift.openCount')}
                  </button>
                </div>
              </>
            }
          >
            <CountForm
              ctx={ctx()}
              kind="OPEN"
              busy={busy}
              setBusy={setBusy}
              onSaved={async () => {
                await staff.refresh();
                await counts.reload();
                setCounting(false);
              }}
              onCancel={() => setCounting(false)}
              onError={setWriteError}
            />
          </Show>
        )}
      </Show>
    </div>
  );

  return { open, Panel };
}
