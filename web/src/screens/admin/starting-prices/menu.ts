import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { StartingPrices } from './StartingPrices';
import * as table from './i18n';

const t = screenT('admin.starting-prices', table);

// SP-3 (#1617): admin starting-prices screen (design `docs/design/blueprint/rates.md` §12.6, §12.7, §12.10 SP-3).
export const menu: AdminMenu = {
  id: 'starting-prices',
  order: 22,
  group: 'money',
  needs: 'OrgStartingPricesManage',
  dated: false,
  icon: '🏷',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: StartingPrices,
  blueprint: true,
};
