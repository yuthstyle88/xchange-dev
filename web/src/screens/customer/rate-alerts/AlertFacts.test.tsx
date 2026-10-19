// C11 behavior tests: AlertFacts (#3449 §8 Behaviour table).
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../../shared/i18n';
import { AlertFacts, type AlertFactsProps } from './AlertFacts';
import * as table from './i18n';

function alert(over: Partial<AlertFactsProps['alert']> = {}): AlertFactsProps['alert'] {
  return {
    id: 'alt-1',
    version: 1,
    currency: 'USD',
    customerSide: 'CUSTOMER_BUY',
    condition: 'ABOVE',
    target: '35.00',
    enabled: true,
    branch: 'BKK-1',
    unitSize: '1',
    currentRate: '34.50',
    ratePublishedAt: '2026-10-19T10:00:00Z',
    ...over,
  };
}

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

function mount(a: AlertFactsProps['alert']) {
  host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <AlertFacts alert={a} />, host);
}

const alertExpired = () => host.querySelector('[data-testid="alert-expired"]');
const alertExpires = () => host.querySelector('[data-testid="alert-expires"]');
const alertStale = () => host.querySelector('[data-testid="alert-stale"]');
const alertRelation = () => host.querySelector('[data-testid="alert-relation"]');
const alertLastEvent = () => host.querySelector('[data-testid="alert-last-event"]');

beforeEach(() => setLang('en'));
afterEach(() => {
  dispose?.();
  dispose = undefined;
  host?.remove();
});

describe('AlertFacts (C11 #3449)', () => {
  it('expired: true: alert-expired shown (key expired); no alert-expires', () => {
    mount(alert({ expired: true, expiresAt: '2026-10-20T10:00:00Z' }));
    expect(alertExpired()).not.toBeNull();
    expect(alertExpired()!.textContent).toContain(table.en.expired);
    expect(alertExpires()).toBeNull();
  });

  it('expired: false, expiresAt set: alert-expires shows the time (key expiresOn)', () => {
    mount(alert({ expired: false, expiresAt: '2026-10-20T10:00:00Z' }));
    expect(alertExpired()).toBeNull();
    expect(alertExpires()).not.toBeNull();
    expect(alertExpires()!.textContent).toContain('2026');
  });

  it('expiresAt: null: neither line', () => {
    mount(alert({ expired: false, expiresAt: null }));
    expect(alertExpired()).toBeNull();
    expect(alertExpires()).toBeNull();
  });

  it('rateStale: true: alert-stale shown', () => {
    mount(alert({ rateStale: true }));
    expect(alertStale()).not.toBeNull();
    expect(alertStale()!.textContent).toContain(table.en.rateStale);
  });

  it('rateStale: false or null: no alert-stale', () => {
    mount(alert({ rateStale: false }));
    expect(alertStale()).toBeNull();

    dispose?.();
    host.remove();

    mount(alert({ rateStale: null }));
    expect(alertStale()).toBeNull();
  });

  it('relation: SATISFIED / NOT_SATISFIED: alert-relation with relationSatisfied / relationNotSatisfied', () => {
    mount(alert({ relation: 'SATISFIED' }));
    expect(alertRelation()).not.toBeNull();
    expect(alertRelation()!.textContent).toContain(table.en.relationSatisfied);

    dispose?.();
    host.remove();

    mount(alert({ relation: 'NOT_SATISFIED' }));
    expect(alertRelation()).not.toBeNull();
    expect(alertRelation()!.textContent).toContain(table.en.relationNotSatisfied);
  });

  it('relation: UNKNOWN: no alert-relation', () => {
    mount(alert({ relation: 'UNKNOWN' }));
    expect(alertRelation()).toBeNull();
  });

  it('lastEvent set: alert-last-event with its time and rate', () => {
    mount(
      alert({
        lastEvent: {
          observedRate: '35.50',
          triggerAt: '2026-10-19T12:00:00Z',
          episodeNo: 1,
        },
      }),
    );
    expect(alertLastEvent()).not.toBeNull();
    const text = alertLastEvent()!.textContent ?? '';
    expect(text).toContain('35.50');
    expect(text).toContain('2026');
  });

  it('lastEvent: null: no alert-last-event', () => {
    mount(alert({ lastEvent: null }));
    expect(alertLastEvent()).toBeNull();
  });

  it('none of the five fields (today backend): renders nothing', () => {
    mount(alert({}));
    expect(host.textContent?.trim()).toBe('');
  });
});
