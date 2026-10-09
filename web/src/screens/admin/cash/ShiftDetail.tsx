import type { JSX } from 'solid-js';
import { For, Show } from 'solid-js';
import type { AdminShiftRow } from '../../../shared/api/generated/admin_shifts';
import { screenT } from '../../../shared/i18n';
import { formatMoney } from '../../../shared/ui/MoneyText';
import { ApiClient } from '../../../shared/api/client';
import { closeDialog, openDialog } from '../parts/Dialog';
import { ProfitSummaryPanel } from './calculators/ProfitSummaryPanel';
import { ShiftValuationPanel } from './calculators/ShiftValuationPanel';
import * as table from './i18n';

const t = screenT('admin.cash', table);

export function openShiftDetail(row: AdminShiftRow, branch: string): void {
  const api = new ApiClient();
  const isClosed = row.status === 'CLOSED';
  const title = isClosed ? t('detailTitleClosed') : t('detailTitleOpen');

  openDialog(title, (): JSX.Element => (
    <div class="shift-detail-dialog">
      <p class="shift-detail-meta">
        {row.id} · {row.staff}
      </p>

      <table class="shift-detail-table">
        <thead>
          <tr>
            <th>{t('headerCurrency')}</th>
            <th>{t('closeAsExpected')}</th>
            <th>{t('headerCounted')}</th>
            <th>{t('headerDifference')}</th>
          </tr>
        </thead>
        <tbody>
          <For each={row.balances}>
            {(b) => (
              <tr>
                <td>{b.code}</td>
                <td>{formatMoney(b.expected)}</td>
                <td>{isClosed && b.counted != null ? formatMoney(b.counted) : '—'}</td>
                <td>{isClosed && b.difference != null ? formatMoney(b.difference) : '—'}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>

      <Show when={isClosed && row.note.trim() !== ''}>
        <p class="shift-detail-note">{row.note}</p>
      </Show>

      <ShiftValuationPanel api={api} branch={branch} shiftId={row.id} />
      <ProfitSummaryPanel api={api} branch={branch} shiftId={row.id} />

      <div class="dialog-actions">
        <button type="button" onClick={closeDialog}>
          {t('close')}
        </button>
      </div>
    </div>
  ));
}
