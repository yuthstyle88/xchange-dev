import { createSignal } from 'solid-js';
import { render } from 'solid-js/web';
import { describe, expect, it, vi } from 'vitest';
import mainSource from './main.tsx?raw';
import cardsSource from './cards.ts?raw';
import { cards } from './cards';
import type { StaffPageProps } from './StaffPage';
import { ConfirmDialog } from './parts/ConfirmDialog';
import { ReasonDialog } from './parts/ReasonDialog';
import { th } from './i18n';

let capturedProps: StaffPageProps | undefined;

vi.mock('./StaffPage', () => ({
  StaffPage: (props: StaffPageProps) => {
    capturedProps = props;
    return <div data-testid="spy-staff-page" />;
  },
}));

describe('main.tsx wiring and mounting (#575)', () => {
  it('collects cards from glob and passes them to StaffPage, not cards={[]}', () => {
    // 1. cards.ts collects using import.meta.glob('./*/card.ts')
    expect(cardsSource).toContain('./*/card.ts');
    expect(cardsSource).toMatch(/collectCards\s*\(\s*import\.meta\.glob/);

    // 2. main.tsx imports cards from ./cards
    expect(mainSource).toMatch(/import\s*\{[^}]*\bcards\b[^}]*\}\s*from\s*['"]\.\/cards['"]/);

    // 3. main.tsx passes cards={cards} to StaffPage
    expect(mainSource).toMatch(/<StaffPage\b[^>]*\bcards=\{cards\}/);

    // 4. main.tsx must not pass cards={[]} or empty array literal
    expect(mainSource).not.toMatch(/cards=\{\s*\[\s*\]\s*\}/);
  });

  it('mounts StaffPage with cards from cards.ts at runtime', async () => {
    const host = document.createElement('div');
    host.id = 'app';
    document.body.appendChild(host);

    await import('./main');

    expect(capturedProps).toBeDefined();
    expect(capturedProps!.cards).toBe(cards);
    expect(capturedProps!.cards).toBeInstanceOf(Array);
    host.remove();
  });
});

describe('ConfirmDialog and ReasonDialog reactivity & i18n (#575 rework)', () => {
  it('ConfirmDialog opens reactively when open changes from false to true and uses i18n labels', () => {
    const [open, setOpen] = createSignal(false);
    const host = document.createElement('div');
    document.body.appendChild(host);

    const dispose = render(
      () => (
        <ConfirmDialog
          open={open()}
          title="Test Title"
          onConfirm={() => {}}
          onCancel={() => {}}
        />
      ),
      host,
    );

    expect(host.querySelector('dialog')).toBeNull();

    setOpen(true);
    const dialog = host.querySelector('dialog');
    expect(dialog).not.toBeNull();
    expect(dialog?.textContent).toContain(th.cancel);
    expect(dialog?.textContent).toContain(th.confirm);

    dispose();
    host.remove();
  });

  it('ReasonDialog opens reactively when open changes from false to true and uses i18n labels', () => {
    const [open, setOpen] = createSignal(false);
    const host = document.createElement('div');
    document.body.appendChild(host);

    const dispose = render(
      () => (
        <ReasonDialog
          open={open()}
          title="Test Title"
          onConfirm={() => {}}
          onCancel={() => {}}
        />
      ),
      host,
    );

    expect(host.querySelector('dialog')).toBeNull();

    setOpen(true);
    const dialog = host.querySelector('dialog');
    expect(dialog).not.toBeNull();
    expect(dialog?.textContent).toContain(th.cancel);
    expect(dialog?.textContent).toContain(th.confirm);
    const input = host.querySelector('input');
    expect(input?.placeholder).toBe(th.reasonPlaceholder);

    dispose();
    host.remove();
  });
});
