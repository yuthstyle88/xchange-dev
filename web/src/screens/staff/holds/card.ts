import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { HoldsCard } from './HoldsCard';

export const card: StaffCard = {
  id: 'holds',
  order: 85,
  visible: (ctx: StaffCtx): boolean =>
    can(ctx.me, 'CustomerHoldPlace') || can(ctx.me, 'VerificationReview'),
  Component: HoldsCard,
};
