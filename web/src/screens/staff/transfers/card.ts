import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { TransfersCard } from './TransfersCard';

export const card: StaffCard = {
  id: 'transfers',
  order: 60,
  visible(ctx: StaffCtx): boolean {
    return can(ctx.me, 'CashManage');
  },
  Component: TransfersCard,
};
