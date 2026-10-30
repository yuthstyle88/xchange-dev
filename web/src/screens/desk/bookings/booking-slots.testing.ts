// #5568 (DESK-R4 T3, docs/design/fe-desk-round4.md §3 T3): the desk dialogs read their slots from view `booking_slots`.
// Test support, written by the ticket author claude/studio-claude-15; do not edit. `withBookingSlots` wraps a test's
// fake `fetch` so the tests written before #5568 keep their meaning: it answers `GET /api/v3/views/booking_slots` with
// one open slot (available 1) per slot of the bookings the fake last answered for `desk_bookings`, and passes every
// other request to the fake unchanged. A test that pins the slots read itself answers `booking_slots` in its own fake.
import type { DeskBookingItemDto } from '../../../shared/api/generated/desk_bookings';
import type { BookingSlotItemDto } from '../../../shared/api/generated/booking_slots';

type FetchFn = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

function urlOf(input: RequestInfo | URL): URL {
  const raw = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
  return new URL(raw, 'http://localhost');
}

export function withBookingSlots(inner: FetchFn): FetchFn {
  let items: DeskBookingItemDto[] = [];
  return async (input, init) => {
    const u = urlOf(input);
    const method = (init?.method ?? 'GET').toUpperCase();
    if (method === 'GET' && u.pathname === '/api/v3/views/booking_slots') {
      const seen = new Set<string>();
      const slots: BookingSlotItemDto[] = [];
      for (const b of items) {
        if (seen.has(b.slotId)) continue;
        seen.add(b.slotId);
        slots.push({ id: b.slotId, start: b.startAt, end: b.endAt, available: 1 });
      }
      return new Response(
        JSON.stringify({ version: 1, changed: true, data: { asOf: '2026-10-30T00:00:00Z', slots } }),
        { status: 200, headers: { 'content-type': 'application/json' } },
      );
    }
    const res = await inner(input, init);
    if (method === 'GET' && u.pathname === '/api/v3/views/desk_bookings' && res.ok) {
      try {
        const body = (await res.clone().json()) as { data?: { items?: DeskBookingItemDto[] } };
        if (Array.isArray(body?.data?.items)) items = body.data.items;
      } catch {
        // not JSON: keep the last items
      }
    }
    return res;
  };
}
