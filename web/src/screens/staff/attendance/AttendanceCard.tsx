import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { fieldError } from '../../../shared/i18n/field-error';
import { isServerFailure, wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { OpenAttendanceRow } from '../../../shared/api/generated/open_attendance';
import type { StaffAttendanceRow } from '../../../shared/api/generated/staff_attendance';
import type { AttendanceClosed, CloseForInput } from '../../../shared/api/generated/staff_corrections';
import type { StaffCtx } from '../cards';
import * as staffI18n from '../i18n';
import { can } from '../parts/can';
import { command } from '../parts/command';
import { attendanceLines } from './attendance-rows';
import { checkCloseFor, defaultClockOutLocal, formatOpenClockIn, toRfc3339Bangkok } from './close-for';
import { th, en } from './i18n';
import './attendance.css';

export interface AttendanceCardProps {
  ctx: StaffCtx;
}

export function AttendanceCard(props: AttendanceCardProps): JSX.Element {
  const t = screenT('staff.attendance', {
    th: { ...staffI18n.th, ...th },
    en: { ...staffI18n.en, ...en },
  });
  const api = new ApiClient();
  const [rows, setRows] = createSignal<StaffAttendanceRow[]>([]);
  const [openRows, setOpenRows] = createSignal<OpenAttendanceRow[]>([]);
  const [error, setError] = createSignal<unknown>(null);

  const [closingId, setClosingId] = createSignal<string | null>(null);
  const [clockOutLocal, setClockOutLocal] = createSignal<string>('');
  const [reason, setReason] = createSignal<string>('');
  const [clockOutError, setClockOutError] = createSignal<string | null>(null);
  const [reasonError, setReasonError] = createSignal<string | null>(null);
  const [formError, setFormError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal<boolean>(false);

  const closeCmds = new Map<string, ReturnType<typeof command<CloseForInput, AttendanceClosed>>>();
  function getCloseCmd(id: string) {
    let c = closeCmds.get(id);
    if (!c) {
      c = command<CloseForInput, AttendanceClosed>(api, 'POST', `/api/attendance/${id}/close`);
      closeCmds.set(id, c);
    }
    return c;
  }

  const mayCloseFor = () => can(props.ctx.me, 'AttendanceCloseFor');
  let reading = false;

  async function loadOpen(): Promise<void> {
    if (!mayCloseFor()) return;
    try {
      const res = await api.view<OpenAttendanceRow[]>('open_attendance');
      if (res.data) {
        setOpenRows(res.data);
      }
    } catch {
      // keep existing openRows if refresh fails
    }
  }

  async function load(): Promise<void> {
    if (reading) return;
    reading = true;
    try {
      const [res] = await Promise.all([
        api.view<StaffAttendanceRow[]>('staff_attendance'),
        loadOpen(),
      ]);
      if (res.data) {
        setRows(res.data);
      }
      setError(null);
    } catch (e: unknown) {
      setError(e);
    } finally {
      reading = false;
    }
  }

  onMount(() => {
    void load();
  });

  function openCloseForm(row: OpenAttendanceRow): void {
    setClosingId(row.id);
    setClockOutLocal(defaultClockOutLocal(row.clockIn));
    setReason('');
    setClockOutError(null);
    setReasonError(null);
    setFormError(null);
  }

  async function handleSubmit(row: OpenAttendanceRow): Promise<void> {
    if (busy()) return;
    setClockOutError(null);
    setReasonError(null);
    setFormError(null);

    const timeFault = checkCloseFor(row.clockIn, clockOutLocal(), new Date());
    const trimmedReason = reason().trim();
    if (timeFault) {
      setClockOutError(t(timeFault.code));
    }
    if (!trimmedReason) {
      setReasonError(t('reasonRequired'));
    }
    if (timeFault || !trimmedReason) {
      return;
    }

    setBusy(true);
    try {
      const payload = {
        branch: props.ctx.me.branchId,
        id: row.id,
        clockOut: toRfc3339Bangkok(clockOutLocal()),
        reason: trimmedReason,
      } satisfies CloseForInput;
      await getCloseCmd(row.id).send(payload);
      setOpenRows((prev) => prev.filter((r) => r.id !== row.id));
      setClosingId(null);
      const res = await api.view<StaffAttendanceRow[]>('staff_attendance');
      if (res.data) {
        setRows(res.data);
      }
      await props.ctx.refresh();
    } catch (err: unknown) {
      const fe = fieldError(err, { clockOut: t('clockOut'), reason: t('reason') });
      const msg =
        err instanceof ApiError &&
        !isServerFailure(err) &&
        Object.prototype.hasOwnProperty.call(th, err.code)
          ? t(err.code as keyof typeof th)
          : fe.message;
      if (fe.field === 'clockOut') {
        setClockOutError(msg);
      } else if (fe.field === 'reason') {
        setReasonError(msg);
      } else {
        setFormError(msg);
      }
      if (err instanceof ApiError && err.code === 'notOpen') {
        void loadOpen();
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <div data-testid="attendance-card" class="xc-attendance-card">
      <div class="xc-attendance-header">
        <h2 class="xc-attendance-title">{t('title')}</h2>
      </div>

      <Show when={error() !== null}>
        <div class="xc-attendance-error-panel">
          <div data-testid="attendance-error" class="xc-attendance-error">{wordedError(error())}</div>
          <button type="button" class="xc-attendance-btn" onClick={() => void load()}>
            {t('refresh')}
          </button>
        </div>
      </Show>

      <div class="xc-attendance-list">
        <For
          each={attendanceLines(rows(), new Date())}
          fallback={<p class="xc-attendance-empty">{t('empty')}</p>}
        >
          {(line) => (
            <div
              data-attendance-id={line.id}
              class="xc-attendance-row"
              classList={{ 'xc-attendance-row-earlier': line.earlierDay }}
            >
              <span class="xc-attendance-name">{line.name}</span>
              <Show when={line.earlierDay}>
                <span class="xc-attendance-earlier-label">{t('earlierDay')}</span>
              </Show>
              <span class="xc-attendance-time">
                {line.stillIn ? (
                  <>
                    {line.inText} <span class="xc-attendance-status">{t('stillIn')}</span>
                  </>
                ) : (
                  `${line.inText} – ${line.outText}`
                )}
              </span>
            </div>
          )}
        </For>
      </div>

      <Show when={mayCloseFor() && openRows().length > 0}>
        <div class="xc-attendance-open-list">
          <For each={openRows()}>
            {(row) => (
              <div data-open-attendance-id={row.id} class="xc-attendance-open-row">
                <div class="xc-attendance-open-summary">
                  <span class="xc-attendance-name">{row.name}</span>
                  <span class="xc-attendance-time">{formatOpenClockIn(row.clockIn)}</span>
                  <button
                    type="button"
                    class="xc-attendance-btn"
                    onClick={() => openCloseForm(row)}
                  >
                    {t('closeFor')}
                  </button>
                </div>

                <Show when={closingId() === row.id}>
                  <form
                    data-testid="attendance-close-form"
                    class="xc-attendance-close-form"
                    onSubmit={(e) => {
                      e.preventDefault();
                      void handleSubmit(row);
                    }}
                  >
                    <div class="xc-attendance-field">
                      <label class="xc-attendance-label">{t('clockOut')}</label>
                      <input
                        type="datetime-local"
                        name="clockOut"
                        data-testid="attendance-close-clock-out"
                        class="xc-attendance-input"
                        value={clockOutLocal()}
                        onInput={(e) => {
                          setClockOutLocal(e.currentTarget.value);
                          setClockOutError(null);
                        }}
                      />
                      <Show when={clockOutError()}>
                        {(msg) => (
                          <div
                            role="alert"
                            data-field-error="clockOut"
                            class="xc-attendance-field-error"
                          >
                            {msg()}
                          </div>
                        )}
                      </Show>
                    </div>

                    <div class="xc-attendance-field">
                      <label class="xc-attendance-label">{t('reason')}</label>
                      <input
                        type="text"
                        name="reason"
                        data-testid="attendance-close-reason"
                        class="xc-attendance-input"
                        maxLength={500}
                        value={reason()}
                        onInput={(e) => {
                          setReason(e.currentTarget.value);
                          setReasonError(null);
                        }}
                      />
                      <Show when={reasonError()}>
                        {(msg) => (
                          <div
                            role="alert"
                            data-field-error="reason"
                            class="xc-attendance-field-error"
                          >
                            {msg()}
                          </div>
                        )}
                      </Show>
                    </div>

                    <Show when={formError()}>
                      {(msg) => (
                        <div
                          role="alert"
                          data-testid="attendance-close-error"
                          class="xc-attendance-form-error"
                        >
                          {msg()}
                        </div>
                      )}
                    </Show>

                    <div class="xc-attendance-form-actions">
                      <button
                        type="button"
                        class="xc-attendance-btn"
                        onClick={() => setClosingId(null)}
                      >
                        {t('cancel')}
                      </button>
                      <button
                        type="submit"
                        data-testid="attendance-close-submit"
                        class="xc-attendance-btn"
                        disabled={busy()}
                      >
                        {t('confirm')}
                      </button>
                    </div>
                  </form>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
