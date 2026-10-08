import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { AppShell } from './AppShell';

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
});

describe('AppShell document order', () => {
  it('renders the nav before the h1 in document order', () => {
    const host = document.createElement('div');
    document.body.appendChild(host);
    dispose = render(
      () => (
        <AppShell
          title="คู่มือพนักงาน XC Desk v2"
          nav={
            <nav>
              <a href="/">กลับแอดมิน</a>
            </nav>
          }
        >
          <p>content</p>
        </AppShell>
      ),
      host,
    );

    const nav = host.querySelector('header nav');
    const h1 = host.querySelector('header h1');

    expect(nav, 'header has nav').not.toBeNull();
    expect(h1, 'header has h1').not.toBeNull();
    expect(
      nav!.compareDocumentPosition(h1!) & Node.DOCUMENT_POSITION_FOLLOWING,
      'nav precedes h1 in document order',
    ).toBeTruthy();
  });
});
