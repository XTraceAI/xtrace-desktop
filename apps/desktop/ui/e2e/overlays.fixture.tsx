import { StrictMode, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, ThemeScope, useTheme } from '../src/theme/ThemeProvider';
import { Popover } from '../src/kit/Popover';
import { Modal } from '../src/kit/Modal';
import '../src/index.css';

function Fixture() {
  const { toggle } = useTheme();
  const [popoverOpen, setPopoverOpen] = useState(false);
  const [modalOpen, setModalOpen] = useState(false);
  const [mounted, setMounted] = useState(true);
  const anchor = useRef<HTMLButtonElement>(null);
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
        <button
          className="refresh-button"
          ref={anchor}
          popoverTarget="details"
          aria-expanded={popoverOpen}
        >
          Open popover
        </button>
        <button className="refresh-button" ref={modalTrigger} onClick={() => setModalOpen(true)}>
          Open modal
        </button>
        {mounted && (
          <>
            <Popover
              id="details"
              open={popoverOpen}
              onOpenChange={setPopoverOpen}
              anchorRef={anchor}
              offset={14}
              aria-label="Details"
              style={{ width: 264 }}
            >
              <h2>Popover details</h2>
              <p>Native top layer, inherited light theme.</p>
              <button
                className="refresh-button"
                popoverTarget="details"
                popoverTargetAction="hide"
                tabIndex={0}
              >
                Close popover
              </button>
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
              <button className="refresh-button" onClick={() => setModalOpen(false)}>
                Close modal
              </button>
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
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <Fixture />
    </ThemeProvider>
  </StrictMode>,
);
