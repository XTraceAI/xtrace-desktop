import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Sidebar, type SidebarKey } from '../src/kit/Sidebar';
import { BrandMark } from '../src/kit/BrandMark';
import { ThemeProvider, useTheme } from '../src/theme/ThemeProvider';
import '../src/index.css';

function Fixture() {
  const { theme, toggle } = useTheme();
  const [activeKey, setActiveKey] = useState<SidebarKey>('dashboard');
  const [inset, setInset] = useState(false);
  return (
    <div style={{ display: 'flex', gap: 40 }}>
      <div
        style={{ width: 228, height: 900, background: 'var(--canvas)' }}
        data-testid="sidebar-preview"
      >
        <Sidebar
          activeKey={activeKey}
          onNavigate={setActiveKey}
          rulebookCount={5}
          hosts={[
            { host: 'claude', tokens: 5500000, fillPercent: 100 },
            { host: 'codex', tokens: 400000, fillPercent: 7 },
            { host: 'cursor', tokens: null, fillPercent: 60 },
          ]}
          listener={{ status: 'listening', port: 47421 }}
          version="0.1.0"
          tokensCaption="since reset · Wed"
          updateLabel="up to date"
          onSettings={() => {}}
          theme={theme}
          onToggleTheme={toggle}
          surfaces={[
            { host: 'Codex', surface: 'cli', status: 'capturing' },
            { host: 'Codex', surface: 'desktop', status: 'not-capturing' },
            { host: 'Other host', surface: 'new-surface', status: 'unknown' },
          ]}
          topInset={inset ? 74 : 16}
        />
      </div>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 24, paddingTop: 32 }}>
        {[20, 26, 34].map((size) => (
          <BrandMark key={size} size={size as 20 | 26 | 34} />
        ))}
        <button className="refresh-button" onClick={() => setInset(!inset)}>
          Reserve native controls
        </button>
        <p data-testid="outside">Component preview</p>
      </div>
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
