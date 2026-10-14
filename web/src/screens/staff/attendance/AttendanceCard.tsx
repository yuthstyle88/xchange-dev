import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import { ApiClient } from '../../../shared/api/client';
import type { StaffAttendanceRow } from '../../../shared/api/generated/staff_attendance';
import type { StaffCtx } from '../cards';
import { th, en } from './i18n';

export interface AttendanceCardProps {
  ctx: StaffCtx;
}

export function AttendanceCard(_props: AttendanceCardProps): JSX.Element {
  const t = screenT('staff.attendance', { th, en });
  const api = new ApiClient();
  const [rows, setRows] = createSignal<StaffAttendanceRow[]>([]);
  const [error, setError] = createSignal<string | null>(null);

  onMount(async () => {
    try {
      const res = await api.view<StaffAttendanceRow[]>('staff_attendance');
      if (res.data) {
        setRows(res.data);
      }
    } catch (e: unknown) {
      setError(wordedError(e));
    }
  });

  return (
    <div data-card="attendance">
      <Show when={error()}>
        <div data-testid="attendance-error">{error()}</div>
      </Show>
      <For each={rows()}>
        {(row) => (
          <div data-attendance-id={row.id}>
            <span>{row.name}</span>
            <Show when={!row.out}>
              <span> {t('stillIn')}</span>
            </Show>
          </div>
        )}
      </For>
    </div>
  );
}
