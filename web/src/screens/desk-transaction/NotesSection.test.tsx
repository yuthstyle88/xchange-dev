// F50 (docs/design/backdated-keying.md §16.5): the desk detail notes section
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { NotesSection } from './NotesSection';

let host: HTMLDivElement;
let dispose: (() => void) | undefined;

beforeEach(() => {
  host = document.createElement('div');
  document.body.appendChild(host);
  setLang('th');
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  host.remove();
  setLang('th');
});

describe('NotesSection (#6714 / F50b)', () => {
  it('renders a note with text as written with heading and pre-wrap style', () => {
    const noteText = 'ร้าน #12 · พนักงาน สมชาย\nโทร 081-234-5678';
    dispose = render(() => <NotesSection notes={noteText} />, host);

    const section = host.querySelector('section[data-section="notes"]');
    expect(section).not.toBeNull();

    const heading = host.querySelector('h2.xc-dtx-heading');
    expect(heading?.textContent).toBe('หมายเหตุ');

    const notesEl = host.querySelector<HTMLElement>('[data-testid="tx-notes"]');
    expect(notesEl).not.toBeNull();
    expect(notesEl?.textContent).toBe(noteText);
    expect(notesEl?.style.whiteSpace).toBe('pre-wrap');
  });

  it('renders the English heading when language is en', () => {
    setLang('en');
    dispose = render(() => <NotesSection notes="Backdated transaction note" />, host);

    const heading = host.querySelector('h2.xc-dtx-heading');
    expect(heading?.textContent).toBe('Note');

    const notesEl = host.querySelector<HTMLElement>('[data-testid="tx-notes"]');
    expect(notesEl?.textContent).toBe('Backdated transaction note');
  });

  it('renders nothing when notes is null', () => {
    dispose = render(() => <NotesSection notes={null} />, host);
    expect(host.querySelector('[data-section="notes"]')).toBeNull();
    expect(host.querySelector('[data-testid="tx-notes"]')).toBeNull();
    expect(host.textContent).toBe('');
  });

  it('renders nothing when notes is undefined', () => {
    dispose = render(() => <NotesSection notes={undefined} />, host);
    expect(host.querySelector('[data-section="notes"]')).toBeNull();
    expect(host.querySelector('[data-testid="tx-notes"]')).toBeNull();
    expect(host.textContent).toBe('');
  });

  it('renders nothing when notes is empty string', () => {
    dispose = render(() => <NotesSection notes="" />, host);
    expect(host.querySelector('[data-section="notes"]')).toBeNull();
    expect(host.querySelector('[data-testid="tx-notes"]')).toBeNull();
    expect(host.textContent).toBe('');
  });

  it('renders nothing when notes is blank / whitespace only', () => {
    dispose = render(() => <NotesSection notes={'   \n\t   '} />, host);
    expect(host.querySelector('[data-section="notes"]')).toBeNull();
    expect(host.querySelector('[data-testid="tx-notes"]')).toBeNull();
    expect(host.textContent).toBe('');
  });
});
