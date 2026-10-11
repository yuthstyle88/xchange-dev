import type { JSX } from 'solid-js';
import type { StaffMeView } from '../../shared/api/generated/staff_me';
import type { StaffFloorView } from '../../shared/api/generated/staff_floor';

export type StaffMe = StaffMeView;
export type StaffFloor = StaffFloorView;

export type StaffCtx = {
  me: StaffMe;
  floor: StaffFloor | undefined;
  refresh: () => Promise<void>;
};

export interface StaffCard {
  id: string;
  order: number;
  visible(ctx: StaffCtx): boolean;
  Component: (p: { ctx: StaffCtx }) => JSX.Element;
}

export function collectCards(mods: Record<string, { card: StaffCard }>): StaffCard[] {
  return Object.values(mods)
    .map((m) => m.card)
    .sort((a, b) => a.order - b.order || a.id.localeCompare(b.id));
}

export const cards = collectCards(
  import.meta.glob<{ card: StaffCard }>('./*/card.ts', { eager: true })
);
