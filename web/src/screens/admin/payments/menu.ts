// V2-payments (#470): payments menu record (§7, design §12.3)

import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { en, th } from './i18n';
import { Payments } from './Payments';

const t = screenT('admin.payments', { th, en });

export const menu: AdminMenu = {
  id: 'payments',
  label: () => t('label'),
  icon: '▣',
  order: 6,
  group: 'money',
  dated: false,
  needs: 'all',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Payments,
};
