import { cleanup, render } from '@testing-library/react';
import { createRef, StrictMode, useRef } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Popover } from './Popover';

const disconnect = vi.fn();
const observe = vi.fn();
beforeEach(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe = observe;
      disconnect = disconnect;
    },
  );
  vi.spyOn(HTMLElement.prototype, 'matches').mockImplementation(function (
    this: HTMLElement,
    selector,
  ) {
    return selector === ':popover-open' && this.dataset.open === 'true';
  });
  Object.defineProperty(HTMLElement.prototype, 'showPopover', {
    configurable: true,
    value: vi.fn(function (this: HTMLElement) {
      this.dataset.open = 'true';
    }),
  });
  Object.defineProperty(HTMLElement.prototype, 'hidePopover', {
    configurable: true,
    value: vi.fn(function (this: HTMLElement) {
      this.dataset.open = 'false';
    }),
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  disconnect.mockClear();
  observe.mockClear();
});

it('positions from the anchor, closes through the native API, and balances listeners under StrictMode', () => {
  const anchor = document.createElement('button');
  document.body.append(anchor);
  anchor.getBoundingClientRect = () => ({ left: 20, right: 50, top: 30, bottom: 60 }) as DOMRect;
  const anchorRef = createRef<HTMLElement>();
  anchorRef.current = anchor;
  const add = vi.spyOn(window, 'addEventListener');
  const remove = vi.spyOn(window, 'removeEventListener');
  const props = { id: 'menu', anchorRef, onOpenChange: vi.fn() };
  const view = render(
    <StrictMode>
      <Popover {...props} open>
        Content
      </Popover>
    </StrictMode>,
  );
  expect(view.getByText('Content').style.top).toBe('68px');
  expect(view.getByText('Content').style.left).toBe('20px');
  view.rerender(
    <StrictMode>
      <Popover {...props} open={false}>
        Content
      </Popover>
    </StrictMode>,
  );
  expect(HTMLElement.prototype.hidePopover).toHaveBeenCalledOnce();
  view.unmount();
  for (const event of ['resize', 'scroll']) {
    expect(remove.mock.calls.filter(([name]) => name === event)).toHaveLength(
      add.mock.calls.filter(([name]) => name === event).length,
    );
  }
  expect(add.mock.calls.some(([name]) => ['keydown', 'click', 'pointerdown'].includes(name))).toBe(
    false,
  );
  expect(disconnect).toHaveBeenCalledTimes(2);
  anchor.remove();
});

it.each(['invoker', 'position'])(
  'tracks a replacement %s node through the same ref while open',
  (target) => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: HTMLElement,
    ) {
      const left = Number(this.dataset.left ?? 0);
      return { left, right: left + 30, top: 30, bottom: 60 } as DOMRect;
    });
    function Fixture({ replaced }: { replaced: boolean }) {
      const anchorRef = useRef<HTMLButtonElement>(null);
      const positionRef = useRef<HTMLSpanElement>(null);
      return (
        <>
          <button
            key={target === 'invoker' ? String(replaced) : 'invoker'}
            ref={anchorRef}
            data-testid="invoker"
            data-left={replaced ? 100 : 20}
          >
            Open
          </button>
          {target === 'position' && (
            <span
              key={String(replaced)}
              ref={positionRef}
              data-testid="position"
              data-left={replaced ? 150 : 50}
            />
          )}
          <Popover
            id="replacement"
            anchorRef={anchorRef}
            positionRef={target === 'position' ? positionRef : undefined}
            open
            onOpenChange={() => {}}
          >
            Replacement content
          </Popover>
        </>
      );
    }
    const view = render(<Fixture replaced={false} />);
    const oldTarget = view.getByTestId(target);
    const content = view.getByText('Replacement content');
    expect(content.style.left).toBe(target === 'position' ? '50px' : '20px');
    view.rerender(<Fixture replaced />);
    const replacement = view.getByTestId(target);
    expect(replacement).not.toBe(oldTarget);
    expect(oldTarget.isConnected).toBe(false);
    expect(content.style.left).toBe(target === 'position' ? '150px' : '100px');
    expect(observe).toHaveBeenCalledWith(replacement);
    expect(disconnect).toHaveBeenCalledOnce();
    window.dispatchEvent(new Event('resize'));
    expect(content.dataset.open).toBe('true');
    expect(HTMLElement.prototype.hidePopover).not.toHaveBeenCalled();
  },
);

it.each(['invoker', 'position'])(
  'dismisses when the %s detaches, including a simultaneous controlled close',
  (target) => {
    for (const nextOpen of [false, true]) {
      const anchor = document.createElement('button');
      document.body.append(anchor);
      const anchorRef = createRef<HTMLElement>();
      anchorRef.current = anchor;
      const geometry = document.createElement('span');
      document.body.append(geometry);
      const positionRef = createRef<HTMLElement>();
      positionRef.current = geometry;
      const changed = vi.fn();
      const view = render(
        <Popover
          id="detached"
          anchorRef={anchorRef}
          positionRef={positionRef}
          open
          onOpenChange={changed}
        >
          Detached content
        </Popover>,
      );
      const content = view.getByText('Detached content');
      expect(content.dataset.open).toBe('true');
      if (target === 'invoker') {
        anchor.remove();
        anchorRef.current = null;
      } else {
        geometry.remove();
        positionRef.current = null;
      }
      view.rerender(
        <Popover
          id="detached"
          anchorRef={anchorRef}
          positionRef={positionRef}
          open={nextOpen}
          onOpenChange={changed}
        >
          Detached content
        </Popover>,
      );
      expect(content.dataset.open).toBe('false');
      if (nextOpen) expect(changed).toHaveBeenCalledWith(false);
      view.unmount();
      anchor.remove();
      geometry.remove();
    }
  },
);
