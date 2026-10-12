// V2-customers (#466): customers menu registration (§7, §8, §9; design §12.3)

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Customers } from './Customers';
import * as table from './i18n';

const t = screenT('admin.customers', table);

export const menu: AdminMenu = {
  id: 'customers',
  order: 8,
  group: 'data',
  needs: 'all',
  dated: false,
  icon: '♙',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Customers,
};
