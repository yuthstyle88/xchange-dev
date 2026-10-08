import type { DevicesOverview } from '../../shared/api/generated/devices';
import { screenT } from '../../shared/i18n';
import * as table from './i18n';

const t = screenT('devices', table);

export function deviceNotice(overview: DevicesOverview): { banner: string; drawer: string } {
  const banner = overview.printerKind === 'NETWORK' ? t('bannerNetwork') : t('bannerSimulated');

  let drawer: string;
  if (overview.drawerAgentActive) {
    drawer = t('drawerAgent');
  } else if (overview.printerKind === 'NETWORK' && overview.hasDrawer !== false) {
    drawer = t('drawerNetwork', { printerModel: overview.printerModel });
  } else {
    drawer = t('drawerNone');
  }

  return { banner, drawer };
}
