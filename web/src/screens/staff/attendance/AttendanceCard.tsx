import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
import type { StaffAttendanceRow } from '../../../shared/api/generated/staff_attendance';
import type { StaffCtx } from '../cards';
import * as staffI18n from '../i18n';
import { th, en } from './i18n';
import './attendance.css';

export interface AttendanceCardProps {
  ctx: StaffCtx;
}

export function AttendanceCard(_props: AttendanceCardProps): JSX.Element {
  const t = screenT('staff.attendance', {
    th: { ...staffI18n.th, ...th },
    en: { ...staffI18n.en, ...en },
  });
  const api = new ApiClient();
  const [rows, setRows] = createSignal<StaffAttendanceRow[]>([]);
  const [error, setError] = createSignal<unknown>(null);
  let reading = false;

  async function load(): Promise<void> {
    if (reading) return;
    reading = true;
    try {
      const res = await api.view<StaffAttendanceRow[]>('staff_attendance');
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
        <For each={rows()}>
          {(row) => (
            <div data-attendance-id={row.id} class="xc-attendance-row">
              <span class="xc-attendance-name">{row.name}</span>
              <Show when={!row.out}>
                <span class="xc-attendance-status"> {t('stillIn')}</span>
              </Show>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
