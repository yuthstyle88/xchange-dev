// /admin/pricing (docs/design/blueprint/pricing-tiers.md §9 A-PR; FE pricing-board skeleton, #564). Found by the admin
// shell's menu glob (menus.ts `collectMenus`); shown only to a session holding `ReferenceRatesRead` (§8).
import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Pricing } from './Pricing';
import * as table from './i18n';

const t = screenT('admin.pricing', table);

export const menu: AdminMenu = {
  id: 'pricing',
  order: 29,
  group: 'money',
  needs: 'ReferenceRatesRead',
  dated: false,
  icon: '▤',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Pricing,
  blueprint: true,
};
