import { screenT } from '../../../shared/i18n';
import type { AdminMenu } from '../menus';
import { BookingSlots } from './BookingSlots';
import * as table from './i18n';

const t = screenT('admin.booking-slots', table);

// BK-9 (#1410): admin booking-slots screen (design `docs/design/blueprint/booking-slots.md` §4.4, §9 BK-9).
export const menu: AdminMenu = {
  id: 'booking-slots',
  order: 25,
  group: 'desk',
  needs: 'BranchesManage',
  dated: false,
  icon: '📅',
  label: () => t('label'),
  heading: {
    eyebrow: () => t('eyebrow'),
    title: () => t('title'),
    subtitle: () => t('subtitle'),
  },
  Component: BookingSlots,
  blueprint: true,
};
