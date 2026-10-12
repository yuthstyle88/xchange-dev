import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { DenominationGroups } from './DenominationGroups';
import * as table from './i18n';

const t = screenT('admin.denomination-groups', table);

export const menu: AdminMenu = {
  id: 'denomination-groups',
  label: () => t('label'),
  icon: '💵',
  order: 13,
  group: 'money',
  needs: 'RatesDraft',
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  dated: false,
  Component: DenominationGroups,
};
