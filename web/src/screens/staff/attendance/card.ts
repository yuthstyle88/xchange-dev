import type { StaffCard, StaffCtx } from '../cards';
import { can } from '../parts/can';
import { AttendanceCard } from './AttendanceCard';

export const card: StaffCard = {
  id: 'attendance',
  order: 90,
  visible: (ctx: StaffCtx): boolean => can(ctx.me, 'AttendanceClock'),
  Component: AttendanceCard,
};
