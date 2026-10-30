// The in-page feed of fresh notices (docs/design/fe-desk-round4.md §2, P1). `NoticeBell` is the only publisher: it
// already polls `my_notifications`, so a card that must react to a notice listens here instead of polling again.
// Written whole in the design PR because it is the seam both P1 tickets build on.

/** One notice the bell saw for the first time (its `seq` is above every seq seen before). */
export interface FreshNotice {
  seq: number;
  objectType: string | null;
  objectId: string | null;
}

type Listener = (notice: FreshNotice) => void;

const listeners = new Set<{ objectType: string; fn: Listener }>();

/** Tells every listener of each notice's `objectType`, in the order given. A throwing listener does not stop the rest. */
export function publishFreshNotices(notices: readonly FreshNotice[]): void {
  for (const notice of notices) {
    for (const l of [...listeners]) {
      if (l.objectType !== notice.objectType) continue;
      try {
        l.fn(notice);
      } catch {
        // one card's failure must not hide the notice from the others
      }
    }
  }
}

/** Calls `fn` for each fresh notice of `objectType`; the returned function stops it. */
export function onFreshNotice(objectType: string, fn: Listener): () => void {
  const entry = { objectType, fn };
  listeners.add(entry);
  return () => {
    listeners.delete(entry);
  };
}
