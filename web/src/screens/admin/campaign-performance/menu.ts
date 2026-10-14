// MK-8c (#1646): Admin Campaign Performance menu definition (M04).
import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { CampaignPerformance } from './CampaignPerformance';
import * as table from './i18n';

const t = screenT('admin.campaign-performance', table);

export const menu: AdminMenu = {
  id: 'campaign-performance',
  order: 24,
  group: 'money',
  needs: 'MarketingPerformance',
  dated: true,
  icon: '📊',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: CampaignPerformance,
  blueprint: true,
};
