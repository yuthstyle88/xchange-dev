import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { RateAlertSettings } from './RateAlertSettings';
import * as table from './i18n';

const t = screenT('admin.rate-alert-settings', table);

// AL-6 (#1011): admin rate-alert settings screen. EXECUTIVE only (`needs: 'RateAlertSettingsSave'`).
export const menu: AdminMenu = {
  id: 'rate-alert-settings',
  order: 20,
  group: 'tools',
  needs: 'RateAlertSettingsSave',
  dated: false,
  icon: '⚡',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: RateAlertSettings,
  blueprint: true,
};
