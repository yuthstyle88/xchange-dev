import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { UsersScreen } from './UsersScreen';
import * as table from './i18n';

const t = screenT('admin.users', table);

export const menu: AdminMenu = {
  id: 'users',
  order: 15,
  group: 'data',
  needs: 'UsersManage',
  dated: false,
  icon: '👥',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: UsersScreen,
};
