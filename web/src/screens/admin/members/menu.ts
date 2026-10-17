import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import * as table from './i18n';
import { Members } from './Members';

const t = screenT('admin.members', table);

export const menu: AdminMenu = {
  id: 'members',
  order: 26,
  group: 'data',
  needs: 'MarketingMembers',
  dated: false,
  icon: '👥',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Members,
  blueprint: true,
};

