import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { ShiftCard } from './ShiftCard';

export const card: StaffCard = {
  id: 'shift',
  order: 20,
  visible: (ctx: StaffCtx): boolean =>
    can(ctx.me, 'ShiftOperate') || can(ctx.me, 'ShiftCloseCertify'),
  Component: ShiftCard,
};
