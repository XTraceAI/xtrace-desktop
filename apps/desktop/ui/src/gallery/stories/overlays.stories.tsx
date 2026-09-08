import { useId, useRef, useState } from 'react';
import { Popover, Modal, HubPopover, Button } from '../../kit';
import { story } from '../story';

function OverlayExample({
  kind,
  initial,
  connected = false,
}: {
  kind: 'popover' | 'modal' | 'hub';
  initial: boolean;
  connected?: boolean;
}) {
  const [open, setOpen] = useState(initial);
  const id = useId();
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <div className="gallery-stack">
      <Button ref={anchor} style={{ alignSelf: 'flex-start' }} onClick={() => setOpen(true)}>
        Open illustrative {kind}
      </Button>
      {kind === 'popover' ? (
        <Popover
          id={id}
          open={open}
          onOpenChange={setOpen}
          anchorRef={anchor}
          className="gallery-popover"
          aria-label="Illustrative popover"
        >
          <p>Illustrative native popover content.</p>
          <Button onClick={() => setOpen(false)}>Close example</Button>
        </Popover>
      ) : kind === 'modal' ? (
        <Modal
          open={open}
          onOpenChange={setOpen}
          returnFocusRef={anchor}
          aria-label="Illustrative modal"
        >
          <p>Illustrative native modal content.</p>
          <label>
            Example name <input />
          </label>
          <Button onClick={() => setOpen(false)}>Close example</Button>
        </Modal>
      ) : (
        <HubPopover
          id={id}
          open={open}
          onOpenChange={setOpen}
          anchorRef={anchor}
          connected={connected}
          teamLabel="Example team"
          onConnect={connected ? undefined : () => setOpen(false)}
        />
      )}
    </div>
  );
}
export const overlayStories = [
  ...(['popover', 'modal'] as const).flatMap((kind) =>
    [false, true].map((initial) =>
      story(
        `${kind}/${initial ? 'open' : 'closed'}`,
        [kind === 'modal' ? 'Modal' : 'Popover'],
        [600, 340],
        () => <OverlayExample kind={kind} initial={initial} />,
      ),
    ),
  ),
  ...[false, true].map((connected) =>
    story(`hub/${connected ? 'connected' : 'disconnected'}`, ['HubPopover'], [620, 400], () => (
      <OverlayExample kind="hub" initial connected={connected} />
    )),
  ),
];
