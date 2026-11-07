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
    // Restated for OL-6 L4 (#6537), issuecomment-6688832431, docs/design/xchange-one-look-layout.md §2 Q1: the title
    // leaves the header and is the first child of the content root.
    const h1 = host.querySelector('h1.xc-shell-title');

    expect(nav, 'header has nav').not.toBeNull();
    expect(h1, 'the page has the title').not.toBeNull();
    expect(host.querySelector('header')!.contains(h1), 'the title is not in the header').toBe(false);
    expect(host.querySelector('main.xc-shell-main')!.firstElementChild, 'the title is the first child of the main').toBe(h1);
    expect(
      nav!.compareDocumentPosition(h1!) & Node.DOCUMENT_POSITION_FOLLOWING,
      'nav precedes h1 in document order',
    ).toBeTruthy();
  });
});
