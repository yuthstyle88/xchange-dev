import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Reconciliation } from './Reconciliation';
import * as table from './i18n';

const t = screenT('admin.reconciliation', table);

// FN-1 (#1068): F01, FINANCE's reconcile-and-certify sheet. The API refuses every other role
// (`FinanceReconcile`).
export const menu: AdminMenu = {
  id: 'reconciliation',
  order: 19,
  group: 'money',
  needs: 'FinanceReconcile',
  dated: true,
  icon: '🧾',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Reconciliation,
  blueprint: true,
};
