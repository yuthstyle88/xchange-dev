import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { CashLimits } from './CashLimits';
import * as table from './i18n';

const t = screenT('admin.cash-limits', table);

// Survey row A22 (#564): a branch's count-difference limit (ledger-inventory.md §3.3, Q-2: ADMIN/FINANCE set it).
export const menu: AdminMenu = {
  id: 'cash-limits',
  order: 28,
  group: 'money',
  needs: 'CashLimitsManage',
  dated: false,
  icon: '⚖',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: CashLimits,
  blueprint: true,
};
