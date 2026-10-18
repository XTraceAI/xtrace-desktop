import { useEffect, useState } from 'react';
import { useData } from '../../data/DataProvider';
import { TelemetryDisclosure } from './TelemetryDisclosure';
import { TelemetryPopup } from './TelemetryPopup';
import { useTelemetry } from './telemetry-query';

/** Dialogs and popovers that take Escape before the floating card does. */
const OPEN_LAYERS = '.xt-modal, .xt-popover, [role="dialog"], [role="alertdialog"]';

/**
 * The disclosure wherever the app says it must be shown: on the welcome page,
 * and as a one-time notice in the main window for anyone who has not seen it
 * (an existing install opens past the welcome page, so without the notice it
 * would never be shown and nothing would ever be sent). Once shown it stays
 * until closed, even after the app recorded it, so it can be read.
 * `suppressed` keeps it off screens that show it themselves or are only
 * passing through; `closable` is false where it is part of the page.
 * `floating` shows it as a compact pop-up over the bottom-right corner of the
 * window that takes no room from the page under it and never takes focus;
 * Escape closes it like Close, since it can cover controls a keyboard user
 * reaches. Without it, the full disclosure is part of the page's flow.
 */
export function TelemetryNotice({
  suppressed = false,
  closable = true,
  floating = false,
}: {
  suppressed?: boolean;
  closable?: boolean;
  floating?: boolean;
}) {
  const { source } = useData();
  const { state } = useTelemetry(source.telemetry);
  const [shown, setShown] = useState<'no' | 'open' | 'closed'>('no');
  const due = !suppressed && shown === 'no' && state.data?.show_disclosure === true;
  // Adjusting state while rendering, so the decision is made on the render
  // that first sees it, with no extra frame.
  if (due) setShown('open');
  const escapable = floating && closable && shown === 'open' && !suppressed;
  useEffect(() => {
    if (!escapable) return;
    const close = (event: KeyboardEvent) => {
      // Escape belongs to an open dialog or popover first, and to anything
      // that already handled it.
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      if (document.querySelector(OPEN_LAYERS)) return;
      setShown('closed');
    };
    document.addEventListener('keydown', close);
    return () => document.removeEventListener('keydown', close);
  }, [escapable]);
  if (shown !== 'open' || !source.telemetry || !state.data || suppressed) return null;
  return (
    <div className={floating ? 'xt-telemetry-floating' : 'xt-telemetry-inline'}>
      {floating ? (
        <TelemetryPopup
          controls={source.telemetry}
          state={state.data}
          onClose={closable ? () => setShown('closed') : undefined}
        />
      ) : (
        <TelemetryDisclosure controls={source.telemetry} state={state.data} />
      )}
    </div>
  );
}
