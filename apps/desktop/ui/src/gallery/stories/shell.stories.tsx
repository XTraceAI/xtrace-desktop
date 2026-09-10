import { useState } from 'react';
import { BrandMark, Sidebar, TopBar, HostGlyph, Icon } from '../../kit';
import type { SidebarKey, SidebarProps } from '../../kit/Sidebar';
import type { SelectedRange } from '../../kit/TopBar';
import type { IconName } from '../../kit/icons';
import { story, type StoryProps } from '../story';
import { AutoActivate } from '../AutoActivate';

const iconNames: IconName[] = [
  'dashboard',
  'sessions',
  'prs',
  'rulebook',
  'leaderboard',
  'cloud',
  'moon',
  'gear',
  'share',
  'scan',
  'copy',
  'calendar',
  'merge',
  'lanes',
  'clock',
  'bolt',
  'msg',
  'token',
  'shield',
];
const icons = {
  dashboard: <Icon name="dashboard" />,
  sessions: <Icon name="sessions" />,
  prs: <Icon name="prs" />,
  rulebook: <Icon name="rulebook" />,
  leaderboard: <Icon name="leaderboard" />,
  hub: <Icon name="cloud" />,
  theme: <Icon name="moon" />,
  settings: <Icon name="gear" />,
};
function SidebarExample({
  theme,
  initial = 'dashboard',
  mode = '',
}: StoryProps & { initial?: SidebarKey; mode?: string }) {
  const [active, setActive] = useState(initial);
  const [notice, setNotice] = useState('Illustrative workspace');
  const listener: SidebarProps['listener'] =
    mode === 'plugin-off'
      ? { status: 'off' }
      : mode === 'unknown'
        ? { status: 'unknown' }
        : { status: 'listening', port: 43100 };
  const sidebar = (
    <Sidebar
      activeKey={active}
      onNavigate={setActive}
      rulebookCount={4}
      leaderboardEnabled={mode !== 'disabled'}
      showTeam={mode === 'team'}
      hubConnected={mode === 'team'}
      teamLabel="Example team"
      onConnectHub={() => setNotice('Connection requested')}
      hosts={
        mode === 'unknown'
          ? []
          : [
              { host: 'claude', tokens: 6400, fillPercent: 64, glyph: <HostGlyph host="claude" /> },
              { host: 'codex', tokens: 0, fillPercent: 0, glyph: <HostGlyph host="codex" /> },
              {
                host: 'cursor',
                tokens: mode === 'cursor-unmeasured' ? null : 3600,
                fillPercent: 36,
                glyph: <HostGlyph host="cursor" />,
              },
            ]
      }
      surfaces={
        mode === 'unknown'
          ? []
          : [
              { host: 'Claude', surface: 'example-cli', status: 'capturing' },
              { host: 'Codex', surface: 'example-editor', status: 'not-capturing' },
              { host: 'Cursor', surface: null, status: 'unknown' },
            ]
      }
      listener={listener}
      version="sample"
      theme={theme}
      onToggleTheme={() => setNotice('Theme toggle requested')}
      onSettings={() => setNotice('Settings requested')}
      tokensCaption={notice}
      topInset={mode === 'native-inset' ? 74 : 16}
      icons={icons}
    />
  );
  return mode === 'hub-popover-open' ? (
    <AutoActivate selector='[aria-label="XTrace Hub"]'>{sidebar}</AutoActivate>
  ) : (
    sidebar
  );
}
function TopBarExample({ mode }: { mode: string }) {
  const [range, setRange] = useState<SelectedRange>(mode === 'custom-range' ? 'custom' : '14d');
  const [actions, setActions] = useState(0);
  const action = mode === 'scan' ? 'scan' : mode === 'copy' ? 'copy' : 'share';
  return (
    <TopBar
      crumb="illustrative"
      subcrumb={mode === 'sub-crumb' ? 'example-detail' : undefined}
      showRange={mode !== 'no-range'}
      range={range}
      onRange={setRange}
      onCustomRange={() => setRange('custom')}
      actionLabel={
        mode === 'no-action' ? undefined : `${action} example${actions ? ` · ${actions}` : ''}`
      }
      actionIcon={action}
      onAction={() => setActions((value) => value + 1)}
    />
  );
}
export const shellStories = [
  story('brand/sizes-and-labels', ['BrandMark'], [500, 90], () => (
    <div className="gallery-row gallery-stack">
      {([20, 26, 34] as const).map((size) => (
        <BrandMark key={size} size={size} />
      ))}
      <BrandMark showLabel={false} />
    </div>
  )),
  story('icons/all', ['Icon'], [620, 220], () => (
    <div className="gallery-grid">
      {iconNames.map((name) => (
        <span key={name} className="gallery-row">
          <Icon name={name} />
          {name}
        </span>
      ))}
    </div>
  )),
  story('host-glyph/known-unknown-sizes-stacked', ['HostGlyph'], [460, 320], () => (
    <div className="gallery-grid">
      {(['claude', 'codex', 'cursor', 'unknown-host'] as const).flatMap((host) =>
        ([16, 18, 20, 30] as const).map((size) => (
          <span className="gallery-row" key={`${host}-${size}`}>
            <HostGlyph host={host} size={size} stacked={size === 18} />
            {host} {size}
          </span>
        )),
      )}
    </div>
  )),
  ...(['dashboard', 'sessions', 'prs', 'rulebook', 'leaderboard'] as const).map((initial) =>
    story(`sidebar/active-${initial}`, ['Sidebar'], [228, 900], (props) => (
      <SidebarExample {...props} initial={initial} />
    )),
  ),
  ...[
    'plugin-off',
    'cursor-unmeasured',
    'unknown',
    'disabled',
    'team',
    'native-inset',
    'hub-popover-open',
  ].map((mode) =>
    story(
      `sidebar/${mode}`,
      ['Sidebar'],
      [mode === 'hub-popover-open' ? 530 : 228, 900],
      (props) => <SidebarExample {...props} mode={mode} />,
    ),
  ),
  ...['default', 'no-range', 'scan', 'copy', 'sub-crumb', 'no-action', 'custom-range'].map((mode) =>
    story(`topbar/${mode}`, ['TopBar'], [1200, 44], () => <TopBarExample mode={mode} />),
  ),
];
