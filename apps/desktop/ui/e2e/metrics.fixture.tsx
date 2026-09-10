import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, ThemeScope, useTheme } from '../src/theme/ThemeProvider';
import { MetricCell } from '../src/kit/MetricCell';
import { RuleChip } from '../src/kit/RuleChip';
import { StatTile } from '../src/kit/StatTile';
import { SectionCard } from '../src/kit/SectionCard';
import { count, hours, tokens } from '../src/kit/format';
import '../src/index.css';
import './metrics.css';

function Fixture() {
  const { theme, setPreference } = useTheme();
  const [scopeTheme, setScopeTheme] = useState<'dark' | 'light'>('light');
  const [clicks, setClicks] = useState(0);
  const missing = 'Cursor Agent CLI usage is absent; token totals were not measured.';
  return (
    <div style={{ padding: 32, maxWidth: 1120, margin: 'auto' }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          flexWrap: 'wrap',
          gap: 12,
        }}
      >
        <h1 style={{ fontSize: 24, margin: 0 }}>Metric cards preview</h1>
        <button onClick={() => setPreference(theme === 'dark' ? 'light' : 'dark')}>
          Switch to {theme === 'dark' ? 'light' : 'dark'} theme
        </button>
      </div>
      <p style={{ color: 'var(--secondary)', margin: '8px 0 20px' }}>
        Synthetic examples · Hover or Tab to a card for its definition
      </p>
      <div className="metrics-preview-grid">
        <StatTile
          label="Parallelism"
          ruleId="M-06"
          icon="lanes"
          value={1.8}
          unit="×"
          delta={0.18}
        />
        <StatTile
          label="Turns"
          ruleId="M-03"
          icon="merge"
          value={4639}
          format={count}
          delta={-0.09}
          deltaTone="bad"
        />
        <StatTile
          label="Active time"
          ruleId="M-05"
          icon="clock"
          value={62.3}
          format={hours}
          unit="h"
        />
        <StatTile label="Peak sessions" ruleId="M-06" icon="bolt" value={0} />
        <StatTile
          label="Messages"
          ruleId="M-02"
          icon="msg"
          value={120}
          aside="Long supplementary context that must truncate"
        />
        <StatTile label="Tokens" ruleId="M-04" icon="token" value={15100000} format={tokens} />
        <StatTile label="Verified" ruleId="U-08" icon="shield" value={0} />
        <StatTile
          label="Unmeasured tokens"
          ruleId="M-04"
          icon="token"
          value={null}
          reason={missing}
          delta={0.2}
        />
        <StatTile
          label="Warning override"
          ruleId="M-18"
          icon="shield"
          iconTone="warning"
          value="Partial"
        />
      </div>
      <div className="metrics-section-grid">
        <SectionCard
          title="Measured values"
          ruleId="M-04"
          meta="Caller-provided presentation"
          right={<button tabIndex={0}>Details</button>}
          footer={
            <>
              <span>Synthetic values</span>
              <span>Current rule</span>
            </>
          }
        >
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 16 }}>
            {([10.5, 11, 18, 24, 30] as const).map((size) => (
              <MetricCell key={size} value={62.3} size={size} />
            ))}
          </div>
        </SectionCard>
        <SectionCard
          title="Missing usage"
          headerHeight={36}
          meta="A deliberately long header explanation that must truncate gracefully"
          footer="Missing differs from measured zero"
        >
          <div style={{ display: 'flex', alignItems: 'baseline', gap: 24 }}>
            <MetricCell value={null} reason={missing} size={30} />
            <MetricCell value={0} size={30} />
            <MetricCell value="unknown model" size={11} />
          </div>
        </SectionCard>
      </div>
      <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: 16 }}>
        <label>
          Outside focus{' '}
          <input
            aria-label="Outside focus"
            style={{ border: '1px solid var(--border)', padding: 4, width: 120 }}
          />
        </label>
        <span data-testid="short-rule">
          <RuleChip ruleId="M-06" />
        </span>
        <button tabIndex={0} onClick={() => setClicks(clicks + 1)}>
          Next control
        </button>
        <span data-testid="long-rule">
          <RuleChip ruleId="M-04" />
        </span>
        <span data-testid="coverage-rule">
          <RuleChip ruleId="C-08" size="sm" />
        </span>
      </div>
      <div style={{ position: 'relative', height: 120, marginTop: 16 }}>
        <span data-testid="click-rule">
          <RuleChip ruleId="M-06" />
        </span>
        <button
          tabIndex={0}
          data-testid="under-tooltip"
          onClick={() => setClicks(clicks + 1)}
          style={{
            position: 'absolute',
            left: 0,
            top: 32,
            width: 400,
            maxWidth: '100%',
            height: 72,
            border: '1px solid var(--border)',
            textAlign: 'left',
            padding: 8,
          }}
        >
          Underlying control · clicks {clicks}
        </button>
      </div>
      <ThemeScope
        theme={scopeTheme}
        data-testid="theme-scope"
        style={{
          padding: 8,
          overflow: 'hidden',
          background: 'var(--canvas)',
          color: 'var(--body)',
        }}
      >
        <span data-testid="scope-rule">
          <RuleChip ruleId="M-06" />
        </span>{' '}
        <button onClick={() => setScopeTheme(scopeTheme === 'light' ? 'dark' : 'light')}>
          Switch scoped theme
        </button>
      </ThemeScope>
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
