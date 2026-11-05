import { createEffect, createSignal, For, Index, Show, type JSX } from 'solid-js';
import { ApiClient, ApiError } from '../../../shared/api/client';
import type {
  BranchShiftsData,
  BranchShiftsSaveInput,
} from '../../../shared/api/generated/branch_shifts';
import { keyedWrite } from '../../../shared/api/keyed-write';
import { screenT } from '../../../shared/i18n';
import { wordedError } from '../../../shared/i18n/worded-error';
import * as table from './i18n';
import { coverageGaps, cutoverOf, defaultEnd } from './shift-coverage';

const t = screenT('admin.branches', table);

export interface ShiftDraftRow {
  id: string | null;
  name: string;
  startsAt: string;
  endsAt: string;
  breakStartsAt: string;
  breakEndsAt: string;
  endTouched: boolean;
}

export interface ShiftsEditorProps {
  branchId: string;
  api?: ApiClient;
}

function invalidPlanMessage(reason: string, detail: string): string {
  switch (reason) {
    case 'none':
      return t('shiftErrorNone');
    case 'tooMany':
      return t('shiftErrorTooMany');
    case 'emptyName':
      return t('shiftErrorEmptyName');
    case 'nameTooLong':
      return t('shiftErrorNameTooLong');
    case 'duplicateName':
      return t('shiftErrorDuplicateName');
    case 'duplicateId':
      return t('shiftErrorDuplicateId');
    case 'zeroLength':
      return t('shiftErrorZeroLength');
    case 'spanOver24h':
      return t('shiftErrorSpanOver24h');
    case 'gap': {
      const [from = '', to = ''] = detail.split(/[-–]/);
      if (from && to) {
        return t('shiftsGapWindow', { from, to });
      }
      return t('shiftErrorInvalid');
    }
    case 'breakIncomplete':
      return t('shiftErrorBreakIncomplete');
    case 'breakOutsideShift':
      return t('shiftErrorBreakOutsideShift');
    case 'badTime':
      return t('shiftErrorBadTime');
    default:
      return t('shiftErrorInvalid');
  }
}

export function ShiftsEditor(props: ShiftsEditorProps): JSX.Element {
  const defaultApi = new ApiClient();
  const api = () => props.api ?? defaultApi;
  let writerClient = api();
  let saveShiftsWrite = keyedWrite<BranchShiftsSaveInput, BranchShiftsData>(
    writerClient,
    'PUT',
    '/api/branches/shifts',
  );

  const getWriter = () => {
    const cur = api();
    if (cur !== writerClient) {
      writerClient = cur;
      saveShiftsWrite = keyedWrite<BranchShiftsSaveInput, BranchShiftsData>(
        cur,
        'PUT',
        '/api/branches/shifts',
      );
    }
    return saveShiftsWrite;
  };

  const [rows, setRows] = createSignal<ShiftDraftRow[]>([]);
  const [defaultShiftHours, setDefaultShiftHours] = createSignal<number>(8);
  const [saving, setSaving] = createSignal<boolean>(false);
  const [error, setError] = createSignal<string | null>(null);
  const [rowError, setRowError] = createSignal<{ index: number; message: string } | null>(null);
  const [status, setStatus] = createSignal<string | null>(null);

  let branchSeq = 0;

  const applyData = (data: BranchShiftsData) => {
    setDefaultShiftHours(data.defaultShiftHours);
    setRows(
      data.templates.map((row) => ({
        id: row.id,
        name: row.name,
        startsAt: row.startsAt,
        endsAt: row.endsAt,
        breakStartsAt: row.breakStartsAt ?? '',
        breakEndsAt: row.breakEndsAt ?? '',
        endTouched: true,
      })),
    );
  };

  const loadShifts = async (branchId: string, seq = branchSeq) => {
    try {
      const res = await api().view<BranchShiftsData>('branch_shifts', {
        branch: branchId,
      });
      if (seq !== branchSeq || branchId !== props.branchId) return;
      if (res.data) {
        applyData(res.data);
      }
    } catch {
      // Leave rows empty if view is unavailable
    }
  };

  createEffect(() => {
    const branchId = props.branchId;
    const seq = ++branchSeq;
    setError(null);
    setRowError(null);
    setStatus(null);
    setRows([]);
    if (branchId) {
      void loadShifts(branchId, seq);
    }
  });

  const clearFeedback = () => {
    setError(null);
    setRowError(null);
    setStatus(null);
  };

  const updateRow = (index: number, patch: Partial<ShiftDraftRow>) => {
    clearFeedback();
    setRows((prev) =>
      prev.map((row, i) => {
        if (i !== index) return row;
        const next: ShiftDraftRow = { ...row, ...patch };
        if (patch.startsAt !== undefined && !next.endTouched) {
          next.endsAt = defaultEnd(patch.startsAt, defaultShiftHours());
        }
        return next;
      }),
    );
  };

  const handleAdd = () => {
    clearFeedback();
    const start = '09:00';
    setRows((prev) => [
      ...prev,
      {
        id: null,
        name: '',
        startsAt: start,
        endsAt: defaultEnd(start, defaultShiftHours()),
        breakStartsAt: '',
        breakEndsAt: '',
        endTouched: false,
      },
    ]);
  };

  const handleRemove = (index: number) => {
    clearFeedback();
    setRows((prev) => prev.filter((_, i) => i !== index));
  };

  const cutover = () => cutoverOf(rows());
  const gaps = () => coverageGaps(rows());
  const canSave = () =>
    rows().length > 0 &&
    gaps().length === 0 &&
    rows().every((r) => r.name.trim().length > 0) &&
    !saving();

  const handleSave = async () => {
    const branchId = props.branchId;
    if (!branchId) return;
    const seq = branchSeq;
    clearFeedback();
    setSaving(true);

    const payload: BranchShiftsSaveInput = {
      branch: branchId,
      templates: rows().map((r) => ({
        id: r.id,
        name: r.name,
        startsAt: r.startsAt,
        endsAt: r.endsAt,
        breakStartsAt: r.breakStartsAt.trim() ? r.breakStartsAt.trim() : null,
        breakEndsAt: r.breakEndsAt.trim() ? r.breakEndsAt.trim() : null,
      })),
    };

    try {
      const res = await getWriter().send(payload);
      if (seq !== branchSeq || branchId !== props.branchId) return;
      applyData(res);
      setStatus(t('shiftsSaved'));
    } catch (err: unknown) {
      if (seq !== branchSeq || branchId !== props.branchId) return;
      if (err instanceof ApiError && err.code === 'shiftPlanInvalid') {
        const detailsObj =
          err.details && typeof err.details === 'object'
            ? (err.details as { reason?: string; detail?: string })
            : undefined;
        const reason = detailsObj?.reason ?? err.reason ?? '';
        const detail = detailsObj?.detail ?? '';
        const msg = invalidPlanMessage(reason, detail);
        if (/^\d+$/.test(detail)) {
          setRowError({ index: Number(detail), message: msg });
        } else {
          setError(msg);
        }
      } else {
        setError(wordedError(err));
      }
    } finally {
      if (seq === branchSeq) setSaving(false);
    }
  };

  return (
    <section class="branches-card branches-shifts" data-section="branch-shifts">
      <h2 class="branches-heading">{t('shiftsHeading')}</h2>

      <Show when={cutover()}>
        {(c) => (
          <p class="branches-shifts-cutover" data-shifts-cutover={c()}>
            {t('shiftsCutover', { time: c() })}
          </p>
        )}
      </Show>

      <div class="branches-shifts-bar" data-section="shifts-bar">
        <For each={rows()}>
          {(row) => (
            <span class="branches-shifts-bar-segment" data-shift-segment={row.name}>
              {row.name} {row.startsAt}–{row.endsAt}
            </span>
          )}
        </For>
        <For each={gaps()}>
          {(gap) => (
            <span class="branches-shifts-bar-gap" data-gap={`${gap.from}-${gap.to}`}>
              {t('shiftsGapWindow', { from: gap.from, to: gap.to })}
            </span>
          )}
        </For>
      </div>

      <Show when={error()}>
        <div role="alert" class="branches-form-error" data-shifts-error>
          {error()}
        </div>
      </Show>
      <Show when={status()}>
        <div role="status" class="branches-form-success" data-shifts-status>
          {status()}
        </div>
      </Show>

      <div class="branches-shifts-list">
        <Index each={rows()}>
          {(row, index) => (
            <div class="branches-shifts-row" data-shift-row={index}>
              <div class="branches-shifts-row-fields">
                <input
                  type="text"
                  name={`shift-name-${index}`}
                  class="branches-input"
                  aria-label={t('shiftName')}
                  placeholder={t('shiftName')}
                  maxLength={40}
                  value={row().name}
                  onInput={(e) => updateRow(index, { name: e.currentTarget.value })}
                />
                <input
                  type="time"
                  name={`shift-start-${index}`}
                  class="branches-time-input"
                  aria-label={t('shiftStart')}
                  value={row().startsAt}
                  onInput={(e) => updateRow(index, { startsAt: e.currentTarget.value })}
                  onChange={(e) => updateRow(index, { startsAt: e.currentTarget.value })}
                />
                <span>–</span>
                <input
                  type="time"
                  name={`shift-end-${index}`}
                  class="branches-time-input"
                  aria-label={t('shiftEnd')}
                  value={row().endsAt}
                  onInput={(e) =>
                    updateRow(index, { endsAt: e.currentTarget.value, endTouched: true })
                  }
                  onChange={(e) =>
                    updateRow(index, { endsAt: e.currentTarget.value, endTouched: true })
                  }
                />
                <input
                  type="time"
                  name={`shift-break-start-${index}`}
                  class="branches-time-input"
                  aria-label={t('shiftBreakStart')}
                  value={row().breakStartsAt}
                  onInput={(e) => updateRow(index, { breakStartsAt: e.currentTarget.value })}
                  onChange={(e) => updateRow(index, { breakStartsAt: e.currentTarget.value })}
                />
                <input
                  type="time"
                  name={`shift-break-end-${index}`}
                  class="branches-time-input"
                  aria-label={t('shiftBreakEnd')}
                  value={row().breakEndsAt}
                  onInput={(e) => updateRow(index, { breakEndsAt: e.currentTarget.value })}
                  onChange={(e) => updateRow(index, { breakEndsAt: e.currentTarget.value })}
                />
                <button
                  type="button"
                  class="branches-btn"
                  data-action="shift-remove"
                  onClick={() => handleRemove(index)}
                >
                  {t('shiftsRemove')}
                </button>
              </div>
              <Show when={rowError()?.index === index ? rowError()!.message : null}>
                {(msg) => (
                  <div
                    role="alert"
                    class="branches-form-error"
                    data-row-error={index}
                  >
                    {msg()}
                  </div>
                )}
              </Show>
            </div>
          )}
        </Index>
      </div>

      <div class="branches-actions">
        <button
          type="button"
          class="branches-btn"
          data-action="shifts-add"
          onClick={handleAdd}
        >
          {t('shiftsAdd')}
        </button>
        <button
          type="button"
          class="branches-btn branches-btn-primary"
          data-action="shifts-save"
          disabled={!canSave()}
          onClick={() => void handleSave()}
        >
          {saving() ? t('shiftsSaving') : t('shiftsSave')}
        </button>
      </div>
    </section>
  );
}
