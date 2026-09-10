import { StrictMode, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, ThemeScope, useTheme } from '../src/theme/ThemeProvider';
import { createPopoverHandle, Popover, PopoverClose, PopoverTrigger } from '../src/kit/Popover';
import { Modal, ModalClose } from '../src/kit/Modal';
import '../src/index.css';

function Fixture() {
  const { toggle } = useTheme();
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [mounted, setMounted] = useState(true);
  const [handle] = useState(createPopoverHandle);
  const modalTrigger = useRef<HTMLButtonElement>(null);
  return (
    <main
      style={{
        alignItems: 'stretch',
        minHeight: '160vh',
        width: 720,
        margin: '0 auto',
        gap: 12,
        justifyContent: 'flex-start',
      }}
    >
      <h1>Design foundation</h1>
      <p>Keyboard and theme acceptance fixture</p>
      <button className="refresh-button" onClick={toggle}>
        Toggle page theme
      </button>
      <button className="refresh-button" onClick={() => setMounted(!mounted)}>
        Mount overlays
      </button>
      <ThemeScope
        theme="light"
        className="bg-canvas text-ink"
        style={{ marginTop: 40, padding: 24 }}
      >
        <h2>Light subtree on a dark page</h2>
        <p
          className="bg-surface text-ink font-mono rounded-card shadow-lift px-3 py-2"
          data-testid="utility-probe"
        >
          0123456789
        </p>
        <PopoverTrigger className="refresh-button" handle={handle} tabIndex={0}>
          Open popover
        </PopoverTrigger>
        <button
          className="refresh-button"
          tabIndex={0}
          ref={modalTrigger}
          onClick={() => setModalOpen(true)}
        >
          Open modal
        </button>
        {mounted && (
          <>
            <Popover
              id="details"
              open={popoverOpen}
              onOpenChange={setPopoverOpen}
              handle={handle}
              offset={14}
              aria-label="Details"
              style={{ width: 264 }}
            >
              <h2>Popover details</h2>
              <p>Portalled panel with a scoped light theme.</p>
              <PopoverClose className="refresh-button" tabIndex={0}>
                Close popover
              </PopoverClose>
            </Popover>
            <Modal
              open={modalOpen}
              returnFocusRef={modalTrigger}
              onOpenChange={setModalOpen}
              aria-labelledby="modal-title"
              style={{ width: 380 }}
            >
              <h2 id="modal-title">Modal details</h2>
              <p>Tab stays inside this dialog.</p>
              <label>
                Name{' '}
                <input className="border border-border bg-canvas rounded-panel p-2" name="name" />
              </label>
              <ModalClose className="refresh-button" tabIndex={0}>
                Close modal
              </ModalClose>
            </Modal>
          </>
        )}
      </ThemeScope>
      <ThemeScope theme="dark">
        <p className="bg-surface text-ink p-3 rounded-card" data-testid="dark-scope">
          Always dark
        </p>
      </ThemeScope>
      <p data-testid="outside">Outside the overlays</p>
    </main>
  );
}
function MovingAnchorFixture() {
  const [handle] = useState(createPopoverHandle);
  const [open, setOpen] = useState(false);
  const [revision, setRevision] = useState(0);
  const [positionAnchor, setPositionAnchor] = useState<HTMLSpanElement | null>(null);
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  const separateAnchor = new URLSearchParams(location.search).has('position');
  return (
    <ThemeScope theme={theme}>
      <div
        style={{ transform: 'translate(90px, 90px)', overflow: 'hidden', width: 500, height: 65 }}
      >
        <PopoverTrigger
          id="moving-trigger"
          key={separateAnchor ? 'trigger' : revision}
          handle={handle}
          style={{ marginLeft: revision * 40 }}
        >
          Moving details
        </PopoverTrigger>
        {separateAnchor && (
          <span
            key={revision}
            ref={setPositionAnchor}
            data-testid="position-anchor"
            style={{ display: 'inline-block', width: 16, height: 16, marginLeft: revision * 30 }}
          />
        )}
        <Popover
          handle={handle}
          id="moving-details"
          open={open}
          onOpenChange={setOpen}
          positionAnchor={separateAnchor ? positionAnchor : undefined}
          aria-label="Moving details"
          style={{ width: 264 }}
        >
          <button onClick={() => setRevision(revision + 1)}>Replace anchor</button>
          <button onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>
            Change scope theme
          </button>
          <PopoverClose>Close moving details</PopoverClose>
        </Popover>
      </div>
      <button style={{ marginTop: 300 }} onClick={() => setRevision(revision + 1)}>
        Outside state change
      </button>
    </ThemeScope>
  );
}

function NestedModalFixture() {
  const [open, setOpen] = useState(false);
  const [childOpen, setChildOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const childTrigger = useRef<HTMLButtonElement>(null);
  return (
    <ThemeScope theme="light">
      <button ref={trigger} onClick={() => setOpen(true)}>
        Open parent
      </button>
      <Modal open={open} onOpenChange={setOpen} returnFocusRef={trigger} aria-label="Parent dialog">
        <button ref={childTrigger} onClick={() => setChildOpen(true)}>
          Open child
        </button>
        <ModalClose>Close parent</ModalClose>
        <Modal
          open={childOpen}
          onOpenChange={setChildOpen}
          returnFocusRef={childTrigger}
          aria-label="Child dialog"
        >
          <input aria-label="Child name" />
          <ModalClose>Close child</ModalClose>
        </Modal>
      </Modal>
    </ThemeScope>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      {location.search.includes('anchors') ? (
        <MovingAnchorFixture />
      ) : location.search.includes('nested') ? (
        <NestedModalFixture />
      ) : (
        <Fixture />
      )}
    </ThemeProvider>
  </StrictMode>,
);
