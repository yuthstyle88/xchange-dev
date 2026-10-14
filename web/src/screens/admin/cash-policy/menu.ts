import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { CashPolicy } from './CashPolicy';
import * as table from './i18n';

const t = screenT('admin.cash-policy', table);

// CA-9b #1372: A5's cash-transfer limit policy (design `blueprint/cash-shift-transfer.md` §9 CA-9b), beside cash.
export const menu: AdminMenu = {
  id: 'cash-policy',
  order: 18,
  group: 'money',
  needs: 'CashTransferPolicySet',
  dated: false,
  icon: '⚖',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: CashPolicy,
  blueprint: true,
};
