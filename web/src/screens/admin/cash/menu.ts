import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Cash } from './Cash';
import * as table from './i18n';

const t = screenT('admin.cash', table);

export const menu: AdminMenu = {
  id: 'cash',
  order: 4,
  group: 'money',
  roles: 'all',
  dated: false,
  icon: '▣',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('headingEyebrow'),
    title: () => t('headingTitle'),
    subtitle: () => t('headingSubtitle'),
  },
  Component: Cash,
};
