import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { BranchesScreen } from './BranchesScreen';
import * as table from './i18n';

const t = screenT('admin.branches', table);

export const menu: AdminMenu = {
  id: 'branches',
  order: 14,
  group: 'data',
  needs: 'BranchesManage',
  dated: false,
  icon: '🏢',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: BranchesScreen,
  blueprint: true,
};
