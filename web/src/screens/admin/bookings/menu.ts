// V2-bookings (#463 §7; design §12.3)
import type { AdminMenu } from '../menus';
import { screenT } from '../../../shared/i18n';
import { th, en } from './i18n';
import { BookingsScreen } from './BookingsScreen';

const t = screenT('admin.bookings', { th, en });

export const menu: AdminMenu = {
  id: 'bookings',
  group: 'desk',
  order: 3,
  icon: '▤',
  dated: false,
  needs: 'all',
  label: () => t('navLabel'),
  heading: {
    eyebrow: () => t('headingEyebrow'),
    title: () => t('navLabel'),
    subtitle: () => t('headingSubtitle'),
  },
  Component: BookingsScreen,
};
