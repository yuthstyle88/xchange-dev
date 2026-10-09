import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Rates } from './Rates';
import * as table from './i18n';

const t = screenT('admin.rates', table);

export const menu: AdminMenu = {
  id: 'rates',
  order: 5,
  group: 'money',
  roles: 'all',
  dated: false,
  icon: '◎',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Rates,
};
