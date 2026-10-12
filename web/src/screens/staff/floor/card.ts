import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { FloorCard } from './FloorCard';

export const card: StaffCard = {
  id: 'floor',
  order: 10,
  visible: (ctx: StaffCtx): boolean => can(ctx.me, 'AttendanceClock'),
  Component: FloorCard,
};
