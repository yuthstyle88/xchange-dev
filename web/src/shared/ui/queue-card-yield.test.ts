// #5133 Y1–Y3: the shared hold that asks the floating queue card to fold while a form is being filled.
// One call to yieldQueueCard() is one hold; its release function drops that hold once (a second call does
// nothing). queueCardYielding() is true while any hold is unreleased, so two holders never release each other.
import { afterEach, describe, expect, it } from 'vitest';
import { queueCardYielding, yieldQueueCard } from './queue-card-yield';

const open: Array<() => void> = [];
function hold(): () => void {
  const release = yieldQueueCard();
  open.push(release);
  return release;
}

afterEach(() => {
  // the module is a page-wide singleton: leave no hold behind for the next test
  for (const release of open.splice(0)) release();
});

describe('#5133 queue-card yield', () => {
  it('Y1: no hold → not yielding', () => {
    expect(queueCardYielding()).toBe(false);
  });

  it('Y2: one hold, then release → true, then false; a second release does nothing', () => {
    const release = hold();
    expect(queueCardYielding()).toBe(true);
    release();
    expect(queueCardYielding()).toBe(false);
    release();
    expect(queueCardYielding(), 'a second release must not go below zero').toBe(false);
    const again = hold();
    expect(queueCardYielding(), 'a new hold after a double release still counts').toBe(true);
    again();
    expect(queueCardYielding()).toBe(false);
  });

  it('Y3: two holds, release one → still yielding; release the other → not', () => {
    const a = hold();
    const b = hold();
    a();
    expect(queueCardYielding()).toBe(true);
    a();
    expect(queueCardYielding(), "releasing A twice must not release B's hold").toBe(true);
    b();
    expect(queueCardYielding()).toBe(false);
  });
});
