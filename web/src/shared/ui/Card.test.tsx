import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { Card, CardHead, CardBody, CardActions, CardGrid } from './Card';
import { StatCard } from './StatCard';

describe('Card components (CD-0, #726)', () => {
  let host: HTMLDivElement;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    host.remove();
  });

  describe('Card', () => {
    it('renders section.card with plain tone by default', () => {
      dispose = render(() => <Card><span>content</span></Card>, host);
      const section = host.querySelector('section');
      expect(section).not.toBeNull();
      expect(section?.className).toBe('card');
      expect(section?.textContent).toBe('content');
    });

    it('renders section.card.dark-card when tone="dark"', () => {
      dispose = render(() => <Card tone="dark"><span>dark content</span></Card>, host);
      const section = host.querySelector('section');
      expect(section?.className).toBe('card dark-card');
    });

    it('appends custom class to section.card', () => {
      dispose = render(() => <Card class="custom-hook"><span>content</span></Card>, host);
      const section = host.querySelector('section');
      expect(section?.className).toBe('card custom-hook');
    });
  });

  describe('CardHead', () => {
    it('renders a div.card-heading and NEVER a <header> element (markup contract §2.2)', () => {
      dispose = render(() => <CardHead title="Test Card" eyebrow="Preview" />, host);
      const heading = host.querySelector('.card-heading');
      expect(heading).not.toBeNull();
      expect(heading?.tagName).toBe('DIV');
      expect(host.querySelector('header')).toBeNull();
    });

    it('renders eyebrow when provided, and defaults title to h2.card-title', () => {
      dispose = render(() => <CardHead title="My Title" eyebrow="My Eyebrow" />, host);
      const eyebrow = host.querySelector('.eyebrow');
      expect(eyebrow).not.toBeNull();
      expect(eyebrow?.tagName).toBe('P');
      expect(eyebrow?.textContent).toBe('My Eyebrow');

      const title = host.querySelector('.card-title');
      expect(title).not.toBeNull();
      expect(title?.tagName).toBe('H2');
      expect(title?.textContent).toBe('My Title');
    });

    it('omits p.eyebrow when eyebrow is omitted', () => {
      dispose = render(() => <CardHead title="No Eyebrow" />, host);
      expect(host.querySelector('.eyebrow')).toBeNull();
      const title = host.querySelector('.card-title');
      expect(title?.tagName).toBe('H2');
      expect(title?.textContent).toBe('No Eyebrow');
    });

    it('renders h3.card-title when level=3', () => {
      dispose = render(() => <CardHead title="Level 3 Title" level={3} />, host);
      const title = host.querySelector('.card-title');
      expect(title?.tagName).toBe('H3');
      expect(title?.textContent).toBe('Level 3 Title');
    });

    it('renders aside element when provided', () => {
      dispose = render(
        () => (
          <CardHead
            title="Title with aside"
            aside={<button type="button">Action</button>}
          />
        ),
        host,
      );
      const asideBtn = host.querySelector('.card-heading button');
      expect(asideBtn).not.toBeNull();
      expect(asideBtn?.textContent).toBe('Action');
    });
  });

  describe('CardBody', () => {
    it('renders div.card-body', () => {
      dispose = render(() => <CardBody><p>Body text</p></CardBody>, host);
      const body = host.querySelector('.card-body');
      expect(body).not.toBeNull();
      expect(body?.tagName).toBe('DIV');
      expect(body?.className).toBe('card-body');
      expect(body?.textContent).toBe('Body text');
    });

    it('renders div.card-body.flush when flush is true', () => {
      dispose = render(() => <CardBody flush><table><tbody><tr><td>Cell</td></tr></tbody></table></CardBody>, host);
      const body = host.querySelector('.card-body');
      expect(body?.className).toBe('card-body flush');
    });
  });

  describe('CardActions', () => {
    it('renders div.card-actions', () => {
      dispose = render(
        () => (
          <CardActions>
            <button type="button">Save</button>
          </CardActions>
        ),
        host,
      );
      const actions = host.querySelector('.card-actions');
      expect(actions).not.toBeNull();
      expect(actions?.tagName).toBe('DIV');
      expect(actions?.querySelector('button')?.textContent).toBe('Save');
    });
  });

  describe('CardGrid', () => {
    it('renders div.card-grid.cols-4 by default', () => {
      dispose = render(() => <CardGrid><div>item</div></CardGrid>, host);
      const grid = host.querySelector('.card-grid');
      expect(grid).not.toBeNull();
      expect(grid?.className).toBe('card-grid cols-4');
    });

    it('renders specified column count', () => {
      dispose = render(() => <CardGrid columns={2}><div>item</div></CardGrid>, host);
      expect(host.querySelector('.card-grid')?.className).toBe('card-grid cols-2');
    });
  });

  describe('StatCard', () => {
    it('renders stat-card markup with money format by default', () => {
      dispose = render(
        () => <StatCard label="Total Sales" value="1234567.89" note="Updated just now" />,
        host,
      );
      const stat = host.querySelector('.stat-card');
      expect(stat).not.toBeNull();
      expect(stat?.querySelector('.stat-label')?.textContent).toBe('Total Sales');
      expect(stat?.querySelector('.stat-value .value')?.textContent).toBe('1,234,567.89');
      expect(stat?.querySelector('.stat-value .unit')?.textContent).toBe('THB');
      expect(stat?.querySelector('.stat-note')?.textContent).toBe('Updated just now');
    });

    it('renders plain unformatted value when format="plain"', () => {
      dispose = render(
        () => <StatCard label="Active Items" value="42" note="Count" format="plain" unit="pcs" />,
        host,
      );
      const stat = host.querySelector('.stat-card');
      expect(stat?.querySelector('.stat-value .value')?.textContent).toBe('42');
      expect(stat?.querySelector('.stat-value .unit')?.textContent).toBe('pcs');
    });
  });
});
