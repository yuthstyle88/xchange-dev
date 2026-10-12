// V2-parts (#476): CSV helpers (ticket §7, §9, design §12.2, §12.8)

import type { ApiClient } from '../../../shared/api/client';
import { sharedSession } from '../../../shared/session/session';

/**
 * EX-9 #1147: whether the signed-in user may export, i.e. `me.permissions` holds `ReportsExport` (E-2: EXECUTIVE and
 * FINANCE). It only hides the export buttons; the server's export gate is the rule.
 */
export function canExport(): boolean {
  return (sharedSession().me()?.permissions ?? []).includes('ReportsExport');
}

export function csvCell(v: unknown): string {
  const s = String(v ?? '');
  const guarded = /^[\s]*[=+@\-]/.test(s) ? "'" + s : s;
  return '"' + guarded.replaceAll('"', '""') + '"';
}

export async function walkPages<Row extends { id: string }, Filter = unknown>(
  api: ApiClient,
  view: string,
  options: { branch: string; filter: Filter },
): Promise<Row[]> {
  const all: Row[] = [];
  let before: string | undefined = undefined;
  while (true) {
    const batch: Row[] = await api.page<Row, Filter>(view, {
      branch: options.branch,
      filter: options.filter,
      limit: 100,
      before,
      // EX-9 #1147: the export walker; every read it makes is an export.
      purpose: 'export',
    });
    all.push(...batch);
    if (batch.length < 100) {
      break;
    }
    before = batch[batch.length - 1]!.id;
  }
  return all;
}

export function downloadCsv(name: string, headers: string[], rows: unknown[][]): void {
  const headerLine = headers.map(csvCell).join(',');
  const rowLines = rows.map((r) => r.map(csvCell).join(','));
  const lines = [headerLine, ...rowLines];
  const content = '\uFEFF' + lines.join('\r\n');
  const blob = new Blob([content], { type: 'text/csv;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 1000);
}
