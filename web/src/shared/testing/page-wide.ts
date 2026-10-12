// QU-6a #1353 (design `docs/design/blueprint/queue-tickets.md` §9, §9.2): the reads the shell makes on every staff and
// admin page, in one place. A screen test answers them from here instead of by hand, so when the shell's queue moves
// (QU-6, to `queue_board`), this file changes and the screen tests do not.

/** The signed-in user, read by `AppShell` on every page. */
export const ME_PATH = '/api/v3/views/me';

/** The queue card's view (`shared/queue/queue-store.ts`), polled on every staff and admin page. */
export const QUEUE_CARD_VIEW = 'kiosk_notifications';
export const QUEUE_CARD_PATH = `/api/v3/views/${QUEUE_CARD_VIEW}`;

/** The queue panel's view (`shared/ui/QueuePanel.tsx`). */
export const QUEUE_PANEL_VIEW = 'kiosk_queue';
export const QUEUE_PANEL_PATH = `/api/v3/views/${QUEUE_PANEL_VIEW}`;

/** The shell's page-wide reads: a screen test answers them and never counts them as the screen's own requests. */
export const PAGE_WIDE: readonly string[] = [ME_PATH, QUEUE_CARD_PATH];

/** The queue card's answer when nothing changed: what every screen test gives it. */
export function queueCardUnchanged(version = 1): { version: number; changed: false } {
  return { version, changed: false };
}
