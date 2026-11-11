import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { CustomerApp } from './CustomerApp';
import * as table from './i18n';

const t = screenT('admin.customer-app', table);

export const menu: AdminMenu = {
  id: 'customer-app',
  order: 31,
  group: 'tools',
  needs: 'SystemSettings',
  dated: false,
  icon: '☏',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: CustomerApp,
  blueprint: true,
};
