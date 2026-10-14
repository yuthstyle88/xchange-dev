import { createSignal, onMount, For, Show, type JSX } from 'solid-js';
import { errorText, screenT } from '../../../shared/i18n';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type { StaffAttendanceRow } from '../../../shared/api/generated/staff_attendance';
import type { StaffCtx } from '../cards';
import { th, en } from './i18n';

export interface AttendanceCardProps {
  ctx: StaffCtx;
}

function toErrorText(e: unknown): string {
  const generic = errorText('serverError', 'เซิร์ฟเวอร์ขัดข้อง');
  if (e instanceof ApiError) {
    if (e.code && e.code !== 'httpError') {
      const text = errorText(e.code, '');
      if (text) return text;
    }
    return generic;
  }
  if (e && typeof e === 'object' && 'code' in e && (e as { code: unknown }).code) {
    const code = String((e as { code: unknown }).code);
    if (code && code !== 'httpError') {
      const text = errorText(code, '');
      if (text) return text;
    }
    return generic;
  }
  if (e instanceof Error && e.message) {
    const text = errorText(e.message, '');
    if (text) return text;
    return generic;
  }
  if (typeof e === 'string' && e) {
    const text = errorText(e, '');
    if (text) return text;
    return generic;
  }
  return generic;
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
      setError(toErrorText(e));
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
