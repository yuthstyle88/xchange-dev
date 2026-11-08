import type { AlertDto } from '../../../shared/api/generated/executive_overview';
import { screenT } from '../../../shared/i18n';
import * as table from './i18n';

const t = screenT('admin.executive', table);

export function alertDetail(a: AlertDto): string {
  switch (a.kind) {
    case 'staleData':
      return a.staleMinutes === null ? '' : t('alertStaleMinutes', { n: a.staleMinutes });
    case 'countDifference':
      return a.differences.map((d) => `${d.currency} ${d.amount}`).join(' · ');
    case 'lowStock':
      return a.stock === null ? '' : t('alertStock', { ...a.stock });
    default:
      return '';
  }
}
