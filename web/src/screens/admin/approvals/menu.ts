import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Approvals } from './Approvals';
import * as table from './i18n';

const t = screenT('admin.approvals', table);

export const menu: AdminMenu = {
  id: 'approvals',
  label: () => t('label'),
  icon: '✓',
  order: 11,
  group: 'tools',
  roles: ['OWNER'],
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: false,
  Component: Approvals,
};
