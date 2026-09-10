import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { TopBar, type SelectedRange } from '../src/kit/TopBar';
import { HostGlyph } from '../src/kit/HostGlyph';
import { Icon, type IconName } from '../src/kit/icons';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

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
];
const longCrumb =
  'A deliberately long workspace breadcrumb that must remain inside the available content area';

function Fixture() {
  const [range, setRange] = useState<SelectedRange>('7d');
  const [action, setAction] = useState('No action');
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: 'minmax(0,1fr)',
        gap: 28,
        width: '100%',
        paddingTop: 24,
      }}
    >
      <div data-testid="design-topbar" style={{ width: 1200, maxWidth: '100%' }}>
        <TopBar
          crumb="Dashboard"
          subcrumb="Overview"
          range={range}
          onRange={setRange}
          onCustomRange={() => setAction('custom picker requested')}
          actionLabel="Share"
          onAction={() => setAction('share')}
        />
      </div>
      <div
        data-testid="minimum-frame"
        style={{ width: 1120, display: 'grid', gridTemplateColumns: '228px minmax(0,1fr)' }}
      >
        <div aria-hidden="true" style={{ background: 'var(--panel)' }} />
        <div data-testid="minimum-topbar">
          <TopBar
            crumb={longCrumb}
            subcrumb={longCrumb}
            range={range}
            onRange={setRange}
            actionIcon="scan"
            actionLabel="Scan"
            onAction={() => setAction('scan')}
          />
        </div>
      </div>
      <div data-testid="optional-topbar" style={{ width: 1200, maxWidth: '100%' }}>
        <TopBar
          crumb="Rulebook"
          showRange={false}
          actionIcon="copy"
          actionLabel="Copy rules"
          onAction={() => setAction('copy')}
        />
      </div>
      <div
        data-testid="glyph-gallery"
        style={{ display: 'grid', gap: 16, width: 660, padding: 20 }}
      >
        <h2>Host glyphs</h2>
        {([16, 18, 20, 30] as const).map((size) => (
          <div
            key={size}
            data-testid={`size-${size}`}
            style={{ display: 'flex', gap: 24, alignItems: 'center' }}
          >
            <span className="mono" style={{ width: 30 }}>
              {size}px
            </span>
            {['claude', 'codex', 'cursor', 'new-host'].map((host) => (
              <HostGlyph key={host} host={host} size={size} />
            ))}
          </div>
        ))}
        <div data-testid="stacked-glyphs">
          <HostGlyph host="claude" size={20} stacked />
          <HostGlyph host="codex" size={20} stacked />
          <HostGlyph host="cursor" size={20} stacked />
        </div>
        <h2>Shared icons</h2>
        <div style={{ display: 'flex', gap: 24 }}>
          {iconNames.map((name) => (
            <span key={name} title={name}>
              <Icon name={name} size={20} />
            </span>
          ))}
        </div>
      </div>
      <div style={{ padding: '0 20px' }}>
        <button type="button" tabIndex={0} onClick={() => setRange('custom')}>
          Confirm custom range
        </button>
      </div>
      <p role="status" style={{ padding: '0 20px' }}>
        {action}
      </p>
    </div>
  );
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <Fixture />
    </ThemeProvider>
  </StrictMode>,
);
