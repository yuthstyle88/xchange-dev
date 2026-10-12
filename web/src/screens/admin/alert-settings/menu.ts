import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { AlertSettings } from './AlertSettings';
import * as table from './i18n';

const t = screenT('admin.alert-settings', table);

export const menu: AdminMenu = {
  id: 'alert-settings',
  order: 16,
  group: 'tools',
  needs: 'AlertSettingsSave',
  dated: false,
  icon: '🔔',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: AlertSettings,
};
