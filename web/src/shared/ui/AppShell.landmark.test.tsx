import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';
import { ReceiptSample } from '../../screens/receipt-sample/ReceiptSample';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

describe('AppShell main landmarks', () => {
  it('without ownsMain, exactly one <main> wraps the children', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => (
        <AppShell title="Test Title">
          <p>child content</p>
        </AppShell>
      ),
      host,
    );

    const mains = host.querySelectorAll('main');
    expect(mains).toHaveLength(1);
    expect(mains[0]?.querySelector('p')?.textContent).toBe('child content');
    expect(host.querySelector('.xc-content')).toBeNull();
  });

  it("with ownsMain, the shell renders no <main> and a child's own <main> is the only one", () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => (
        <AppShell title="Test Title" ownsMain>
          <main class="child-main">child landmark</main>
        </AppShell>
      ),
      host,
    );

    const mains = host.querySelectorAll('main');
    expect(mains).toHaveLength(1);
    expect(mains[0]?.classList.contains('child-main')).toBe(true);
    const content = host.querySelector('div.xc-content');
    expect(content).not.toBeNull();
    expect(content?.querySelector('main.child-main')).not.toBeNull();
  });

  it('the receipt-sample page mounted through its screen has exactly one main element', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(() => <ReceiptSample />, host);

    const mains = host.querySelectorAll('main');
    expect(mains).toHaveLength(1);
    expect(mains[0]?.classList.contains('receipt')).toBe(true);
  });
});
