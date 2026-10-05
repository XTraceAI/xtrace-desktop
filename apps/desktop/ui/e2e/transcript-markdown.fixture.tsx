import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { SessionTranscript } from '../src/app/session-detail/SessionTranscript';
import type { TranscriptRecord } from '../src/app/session-detail/transcript-view';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * Synthetic records only: every string below is written here to exercise a shape the
 * formatted answer has to survive in a real engine — a wide table, a long table, a long fence,
 * a long answer behind the clamp, hostile markup, and a person's prompt that must stay literal.
 */
// Wider and taller than its box, and still short enough to be shown unclamped: long headers
// with no break opportunity make it wide, many short rows make it tall.
const headers = Array.from({ length: 8 }, (_, column) => `syntheticcolumnheader${column + 1}`);
const table = [
  `| ${headers.join(' | ')} |`,
  `| ${headers.map(() => '---').join(' | ')} |`,
  ...Array.from({ length: 16 }, (_, row) => `| ${headers.map(() => row + 1).join(' | ')} |`),
].join('\n');
const fence = [
  '```text',
  ...Array.from({ length: 16 }, (_, line) => `synthetic output line ${line + 1}`),
  '```',
].join('\n');

const records: TranscriptRecord[] = [
  {
    id: 'e2e-user',
    role: 'user',
    blocks: [{ kind: 'text', id: 'e2e-user:0', text: 'Keep **this** `literal`, please.' }],
  },
  {
    id: 'e2e-answer',
    role: 'assistant',
    blocks: [
      {
        kind: 'text',
        id: 'e2e-answer:0',
        text: `## Summary\n\nA **wide** table and a long fence follow.\n\n${table}`,
      },
      { kind: 'text', id: 'e2e-answer:1', text: fence },
      {
        kind: 'text',
        id: 'e2e-answer:2',
        text: 'Markup: <script>window.__ran = 1</script> [go](javascript:window.__ran=2) ![pixel](https://example.invalid/pixel.png) https://example.invalid/bare',
      },
    ],
  },
  {
    id: 'e2e-long',
    role: 'assistant',
    blocks: [
      {
        kind: 'text',
        id: 'e2e-long:0',
        text: [
          '# Long answer',
          ...Array.from({ length: 40 }, (_, n) => `- point ${n + 1} with \`code\``),
          '',
          '| a | b |',
          '| - | - |',
          ...Array.from({ length: 60 }, (_, n) => `| row ${n + 1} | ${n} |`),
          '',
          'Final synthetic sentence.',
        ].join('\n'),
      },
    ],
  },
];

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <main style={{ maxWidth: 720, padding: 16 }}>
        <SessionTranscript records={records} />
      </main>
    </ThemeProvider>
  </StrictMode>,
);
