import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import * as table from './i18n';
import { Reports } from './Reports';

const t = screenT('admin.reports', table);

export const menu: AdminMenu = {
  id: 'reports',
  order: 7,
  group: 'money',
  roles: 'all',
  dated: true,
  icon: '▥',
  label: () => t('report'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('report'),
    subtitle: () => t('subtitle'),
  },
  Component: Reports,
};
