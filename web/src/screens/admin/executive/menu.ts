// EX-4 (#1186 §7): the A01 menu. Shown to whoever holds the permission `OverviewRead` (#1030 §3.10).

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Executive } from './Executive';
import * as table from './i18n';

const t = screenT('admin.executive', table);

export const menu: AdminMenu = {
  id: 'executive',
  order: 17,
  group: 'data',
  needs: 'OverviewRead',
  dated: true,
  icon: '🏢',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Executive,
};
