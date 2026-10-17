// LY-6-FE (#2129): Admin Loyalty Rule Versions menu definition.
import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Loyalty } from './Loyalty';
import * as table from './i18n';

const t = screenT('admin.loyalty', table);

export const menu: AdminMenu = {
  id: 'loyalty',
  order: 27,
  group: 'money',
  needs: 'LoyaltyRulesManage',
  dated: false,
  icon: '⭐',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Loyalty,
  blueprint: true,
};
