// docs/design/fe-desk-round4.md §2 (P1): the fresh-notice feed NoticeBell publishes to.
import { describe, expect, it } from 'vitest';
import { onFreshNotice, publishFreshNotices, type FreshNotice } from './notice-feed';

const n = (seq: number, objectType: string | null): FreshNotice => ({ seq, objectType, objectId: `o-${seq}` });

describe('notice-feed', () => {
  it('F1 a listener hears only its objectType, in the order published', () => {
    const heard: number[] = [];
    const stop = onFreshNotice('cash_count', (x) => heard.push(x.seq));
    publishFreshNotices([n(3, 'cash_count'), n(4, 'booking'), n(5, 'cash_count'), n(6, null)]);
    stop();
    expect(heard).toEqual([3, 5]);
  });

  it('F2 after stop, the listener hears nothing', () => {
    const heard: number[] = [];
    const stop = onFreshNotice('cash_count', (x) => heard.push(x.seq));
    stop();
    publishFreshNotices([n(7, 'cash_count')]);
    expect(heard).toEqual([]);
  });

  it('F3 a throwing listener does not stop the others', () => {
    const heard: number[] = [];
    const stopA = onFreshNotice('cash_count', () => {
      throw new Error('boom');
    });
    const stopB = onFreshNotice('cash_count', (x) => heard.push(x.seq));
    publishFreshNotices([n(8, 'cash_count')]);
    stopA();
    stopB();
    expect(heard).toEqual([8]);
  });
});
