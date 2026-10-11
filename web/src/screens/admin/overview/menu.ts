// V2-overview (#461): overview menu definition (ticket §7, §9; design §12.3)

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { en, th } from './i18n';
import { Overview } from './Overview';

const t = screenT('admin.overview', { th, en });

export const menu: AdminMenu = {
  id: 'overview',
  label: () => t('menuLabel'),
  icon: '▦',
  order: 1,
  group: 'desk',
  needs: 'all',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: true,
  Component: Overview,
};
