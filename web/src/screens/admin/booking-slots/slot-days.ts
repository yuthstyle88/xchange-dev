import type { AdminSlotDto } from '../../../shared/api/generated/admin_booking_slots';

export interface SlotDay {
  date: string;
  slots: AdminSlotDto[];
  open: number;
}

/** One group per `serviceDate`, days in the order they first appear, slots in the server's order. */
export function groupSlotsByDay(slots: readonly AdminSlotDto[]): SlotDay[] {
  const days = new Map<string, SlotDay>();
  for (const slot of slots) {
    let day = days.get(slot.serviceDate);
    if (!day) {
      day = { date: slot.serviceDate, slots: [], open: 0 };
      days.set(slot.serviceDate, day);
    }
    day.slots.push(slot);
    if (slot.status === 'OPEN') day.open += 1;
  }
  return [...days.values()];
}
