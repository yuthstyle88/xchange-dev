import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { RatesCard } from './RatesCard';

export const card: StaffCard = {
  id: 'rates',
  order: 80,
  visible: (ctx: StaffCtx): boolean => can(ctx.me, 'CompatRead'),
  Component: RatesCard,
};
