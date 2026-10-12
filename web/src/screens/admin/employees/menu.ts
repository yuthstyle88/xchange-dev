import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Employees } from './Employees';
import * as table from './i18n';

const t = screenT('admin.employees', table);

export const menu: AdminMenu = {
  id: 'employees',
  order: 10,
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
  Component: Employees,
};
