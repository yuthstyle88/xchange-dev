import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { TransfersCard } from './TransfersCard';

export const card: StaffCard = {
  id: 'transfers',
  order: 60,
  visible(ctx: StaffCtx): boolean {
    return (
      can(ctx.me, 'CashManage') ||
      (can(ctx.me, 'ShiftOperate') && Boolean(ctx.floor?.myShift))
    );
  },
  Component: TransfersCard,
};
