// V2-slips (#465): slips menu definition (ticket §7, §9; design §12.3)

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { en, th } from './i18n';
import { Slips } from './Slips';

const t = screenT('admin.slips', { th, en });

export const menu: AdminMenu = {
  id: 'slips',
  label: () => t('menuLabel'),
  icon: '▧',
  order: 9,
  group: 'data',
  roles: 'all',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: false,
  Component: Slips,
};
