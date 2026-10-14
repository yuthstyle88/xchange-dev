// MK-8a (#1645): Admin Marketing Campaigns menu definition (M01 + M02).
import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { Campaigns } from './Campaigns';
import * as table from './i18n';

const t = screenT('admin.campaigns', table);

export const menu: AdminMenu = {
  id: 'campaigns',
  order: 23,
  group: 'money',
  needs: 'MarketingCampaigns',
  dated: true,
  icon: '📢',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: Campaigns,
};
