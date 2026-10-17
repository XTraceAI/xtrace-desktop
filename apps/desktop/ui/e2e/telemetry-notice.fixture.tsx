import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import type { DataSource, TelemetryControls } from '../src/data/DataSource';
import { loadFixtureDataSource } from '../src/data/FixtureDataSource';
import type { TelemetryState } from '../src/data/generated/TelemetryState';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The F1 app on a chosen route with synthetic telemetry controls: `?notice=1`
 * answers as an install that has not seen the disclosure yet, anything else
 * as one that has. Nothing is saved or sent.
 */
const params = new URLSearchParams(location.search);
const route = params.get('route') ?? '/dashboard';
const unseen = params.get('notice') === '1';
let state: TelemetryState = {
  preference_on: true,
  enabled: true,
  disclosure_shown: !unseen,
  env_off: false,
  configured: true,
  can_send: !unseen,
  can_change: true,
  show_disclosure: unseen,
  status: unseen ? 'awaiting_disclosure' : 'sending',
};
const telemetry: TelemetryControls = {
  read: async () => state,
  setEnabled: async (enabled) => {
    if (!enabled) (window as { telemetryTurnedOff?: boolean }).telemetryTurnedOff = true;
    state = {
      ...state,
      preference_on: enabled,
      enabled,
      can_send: enabled && state.disclosure_shown,
      show_disclosure: false,
      status: enabled ? 'sending' : 'turned_off',
    };
    return state;
  },
  acknowledgeDisclosure: async () => {
    (window as { telemetryShown?: number }).telemetryShown =
      ((window as { telemetryShown?: number }).telemetryShown ?? 0) + 1;
    state = {
      ...state,
      disclosure_shown: true,
      can_send: state.preference_on,
      show_disclosure: false,
      status: state.preference_on ? 'sending' : 'turned_off',
    };
    return state;
  },
  openEventList: async () => {},
};

const root = document.getElementById('root');
if (!root) throw new Error('The fixture root is missing.');
void loadFixtureDataSource('F1').then((fixture) => {
  // The fixture's own reads, plus the telemetry controls it does not have.
  const source = new Proxy(fixture, {
    get: (target, key) => (key === 'telemetry' ? telemetry : Reflect.get(target, key)),
  }) as DataSource;
  createRoot(root).render(
    <StrictMode>
      <ThemeProvider>
        <DataProvider source={source}>
          <MemoryRouter initialEntries={[route]}>
            <AppRoutes />
          </MemoryRouter>
        </DataProvider>
      </ThemeProvider>
    </StrictMode>,
  );
});
