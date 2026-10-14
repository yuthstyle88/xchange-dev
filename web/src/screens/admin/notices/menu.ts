import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import * as table from './i18n';
import { Notices } from './Notices';

const t = screenT('admin.notices', table);

// SM-3 (#1491): admin notices page. Every signed-in admin reads their own notices (`needs: 'all'`).
export const menu: AdminMenu = {
  id: 'notices',
  order: 21,
  group: 'tools',
  needs: 'all',
  dated: false,
  icon: '📢',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Notices,
  blueprint: true,
};
