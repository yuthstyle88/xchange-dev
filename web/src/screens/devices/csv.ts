import type { DeviceEventRow } from '../../shared/api/generated/device_events';
import { screenT } from '../../shared/i18n';
import * as table from './i18n';

const t = screenT('devices', table);

const BOM = '\uFEFF';

function escapeCell(val: unknown): string {
  let s = String(val ?? '');
  if (/^[=+\-@]/.test(s)) {
    s = `'${s}`;
  }
  return `"${s.replaceAll('"', '""')}"`;
}

export function eventsCsv(rows: DeviceEventRow[]): string {
  const header = [t('colTime'), t('colActor'), t('colEvent'), t('colRef'), t('colDetail')]
    .map(escapeCell)
    .join(',');

  if (rows.length === 0) {
    return BOM + header;
  }

  const lines = rows.map((r) =>
    [r.at, r.actor, r.kind, r.ref, r.detail].map(escapeCell).join(','),
  );

  return `${BOM}${header}\r\n${lines.join('\r\n')}`;
}
