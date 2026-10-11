// V2-transactions (#464): transactions menu definition (ticket §7, §9; design §12.3)

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { en, th } from './i18n';
import { Transactions } from './Transactions';

const t = screenT('admin.transactions', { th, en });

export const menu: AdminMenu = {
  id: 'transactions',
  label: () => t('menuLabel'),
  icon: '⇄',
  order: 2,
  group: 'desk',
  needs: 'all',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: true,
  Component: Transactions,
};
