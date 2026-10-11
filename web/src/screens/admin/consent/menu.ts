import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Consent } from './Consent';
import * as table from './i18n';

const t = screenT('admin.consent', table);

export const menu: AdminMenu = {
  id: 'consent',
  label: () => t('label'),
  icon: '📜',
  order: 12,
  group: 'tools',
  needs: 'ConsentPolicyManage',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: false,
  Component: Consent,
};
