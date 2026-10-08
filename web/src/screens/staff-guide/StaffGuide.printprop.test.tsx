import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { setLang } from '../../shared/i18n';
import { StaffGuide, type StaffGuideProps } from './StaffGuide';

const READY = {
  build: '0123456',
  builtAt: '2026-04-10T12:00:00Z',
  channel: 'staging',
  version: '1.0.0',
};

let dispose: (() => void) | undefined;
let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  window.localStorage.clear();
  setLang('th');
  fetchMock = vi.fn(async (input: unknown) => {
    if (String(input).endsWith('/health/ready')) {
      return new Response(JSON.stringify(READY), { status: 200 });
    }
    return new Response('{}', { status: 404 });
  });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = '';
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  setLang('th');
  window.localStorage.clear();
});

function mount(props: StaffGuideProps = {}): HTMLElement {
  const host = document.createElement('div');
  document.body.appendChild(host);
  dispose = render(() => <StaffGuide {...props} />, host);
  return host;
}

function printButton(host: HTMLElement): HTMLButtonElement {
  const btn = host.querySelector<HTMLButtonElement>('nav button');
  if (!btn) throw new Error('no print button in nav');
  return btn;
}

describe('StaffGuide — print prop (#480)', () => {
  it('calls the print prop and never window.print', () => {
    const printSpy = vi.spyOn(window, 'print').mockImplementation(() => {});
    const printProp = vi.fn();

    const host = mount({ print: printProp });
    expect(printProp).toHaveBeenCalledTimes(0);
    expect(printSpy).toHaveBeenCalledTimes(0);

    printButton(host).click();

    expect(printProp).toHaveBeenCalledTimes(1);
    expect(printSpy).toHaveBeenCalledTimes(0);
  });
});
