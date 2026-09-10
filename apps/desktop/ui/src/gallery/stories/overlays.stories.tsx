import { useId, useRef, useState } from 'react';
import { Popover, Modal, HubPopover, Button } from '../../kit';
import { createPopoverHandle, PopoverTrigger, PopoverClose } from '../../kit/Popover';
import { ModalClose } from '../../kit/Modal';
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
  const [handle] = useState(() => createPopoverHandle());
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <div className="gallery-stack">
      {kind === 'modal' ? (
        <Button ref={anchor} style={{ alignSelf: 'flex-start' }} onClick={() => setOpen(true)}>
          Open illustrative modal
        </Button>
      ) : (
        <PopoverTrigger handle={handle} render={<Button />} style={{ alignSelf: 'flex-start' }}>
          Open illustrative {kind}
        </PopoverTrigger>
      )}
      {kind === 'popover' ? (
        <Popover
          id={id}
          open={open}
          onOpenChange={setOpen}
          handle={handle}
          className="gallery-popover"
          aria-label="Illustrative popover"
        >
          <p>Illustrative popover content.</p>
          <PopoverClose render={<Button />}>Close example</PopoverClose>
        </Popover>
      ) : kind === 'modal' ? (
        <Modal
          open={open}
          onOpenChange={setOpen}
          returnFocusRef={anchor}
          aria-label="Illustrative modal"
        >
          <p>Illustrative modal content.</p>
          <label>
            Example name <input />
          </label>
          <ModalClose render={<Button />}>Close example</ModalClose>
        </Modal>
      ) : (
        <HubPopover
          id={id}
          open={open}
          onOpenChange={setOpen}
          handle={handle}
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
