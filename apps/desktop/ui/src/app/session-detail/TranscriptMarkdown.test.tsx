import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { SessionTranscript } from './SessionTranscript';
import { CLAMP_CHARACTERS, CLAMP_LINES, clampPreview } from './TranscriptText';
import type { RevealOutcome } from './transcript-reveal';
import type { TranscriptRecord, TranscriptRole } from './transcript-view';

/**
 * The agent's answer as Markdown, and everything else as the characters it is.
 *
 * Every string here is written by this test. None came from a session.
 */

afterEach(cleanup);

const transcript = () => screen.getByRole('region', { name: 'Session transcript' });

const FORMATTED = [
  '## Findings',
  '',
  'The retry loop is **unbounded** and _quiet_; run `pnpm verify` to see it.',
  '',
  '- first point',
  '- second point',
  '',
  '1. step one',
  '2. step two',
  '',
  '> quoted line',
  '',
  '| name | count |',
  '| :--- | ----: |',
  '| alpha | 17 |',
  '',
  '```ts',
  'const answer = 42;',
  '```',
].join('\n');

function textRecord(id: string, role: TranscriptRole, text: string): TranscriptRecord {
  return { id, role, blocks: [{ kind: 'text', id: `${id}:0`, text }] };
}

const blockNode = (id: string) => transcript().querySelector(`[data-block-id="${id}"]`)!;

it('formats the agent’s answer: headings, emphasis, lists, quotes, inline code, tables, fences', () => {
  render(<SessionTranscript records={[textRecord('a1', 'assistant', FORMATTED)]} />);
  const block = blockNode('a1:0');

  // The page is an h1 and its sections h2, so an answer's `##` sits under them as an h4.
  const heading = block.querySelector('h4')!;
  expect(heading.textContent).toBe('Findings');
  expect(heading.getAttribute('data-level')).toBe('2');
  expect(block.querySelector('h1, h2')).toBeNull();
  expect(block.querySelector('strong')?.textContent).toBe('unbounded');
  expect(block.querySelector('em')?.textContent).toBe('quiet');
  expect(block.querySelector('code.xt-md-code')?.textContent).toBe('pnpm verify');
  expect([...block.querySelectorAll('ul > li')].map((li) => li.textContent)).toEqual([
    'first point',
    'second point',
  ]);
  expect(block.querySelectorAll('ol > li')).toHaveLength(2);
  expect(block.querySelector('blockquote')?.textContent?.trim()).toBe('quoted line');

  const table = within(block as HTMLElement).getByRole('region', { name: 'Table, scrollable' });
  expect(table.tabIndex).toBe(0);
  expect([...table.querySelectorAll('th')].map((th) => th.textContent)).toEqual(['name', 'count']);
  expect((table.querySelectorAll('td')[1] as HTMLElement).style.textAlign).toBe('right');

  // A fence is the transcript's own code surface: literal, bounded, and a keyboard stop.
  const code = within(block as HTMLElement).getByRole('region', { name: 'ts, scrollable code' });
  expect(code.tagName).toBe('PRE');
  expect(code.tabIndex).toBe(0);
  expect(code.textContent).toBe('const answer = 42;');
  expect(code.querySelector('*')).toBeNull();

  expect(screen.queryByText(/## Findings/)).toBeNull();
});

it('keeps a person’s prompt, a system line, thinking, tool payloads and code as typed', () => {
  const records: TranscriptRecord[] = [
    textRecord('u1', 'user', FORMATTED),
    textRecord('s1', 'system', FORMATTED),
    {
      id: 'a2',
      role: 'assistant',
      blocks: [
        { kind: 'thinking', id: 'a2:0', text: '**not emphasis** in thinking' },
        { kind: 'code', id: 'a2:1', label: 'md', text: '# not a heading in code' },
        {
          kind: 'tool_call',
          id: 'a2:2',
          name: 'Bash',
          callId: 'call-md-1',
          input: { kind: 'text', text: '**not emphasis** in input' },
          outcome: { state: 'ok', output: { kind: 'text', text: '# not a heading in output' } },
        },
      ],
    },
  ];
  render(<SessionTranscript records={records} />);

  for (const id of ['u1:0', 's1:0']) {
    const block = blockNode(id);
    expect(block.querySelector('[data-format="markdown"]')).toBeNull();
    expect(block.querySelector('h3, h4, h5, h6, strong, em, ul, ol, table, blockquote')).toBeNull();
    // Every character the person typed, including the ones Markdown would have eaten.
    expect(block.querySelector('p')?.textContent).toBe(FORMATTED);
  }

  expect(blockNode('a2:0').textContent).toBe('**not emphasis** in thinking');
  expect(blockNode('a2:0').querySelector('strong')).toBeNull();
  expect(blockNode('a2:1').querySelector('pre')?.textContent).toBe('# not a heading in code');

  fireEvent.click(screen.getByRole('button', { name: /Ran 1 command/ }));
  const card = transcript().querySelector('[data-call-id="call-md-1"]')!;
  expect(card.querySelector('[data-payload="input"] pre')?.textContent).toBe(
    '**not emphasis** in input',
  );
  expect(card.querySelector('[data-payload="output"] pre')?.textContent).toBe(
    '# not a heading in output',
  );
  expect(card.querySelector('strong, h3, h4, h5, h6')).toBeNull();
  expect(transcript().querySelectorAll('[data-format="markdown"]')).toHaveLength(0);
});

const HOSTILE = [
  'Raw markup: <script>window.__ran = 1</script> <img src="https://example.invalid/raw.png" onerror="window.__ran = 2">',
  '',
  '<iframe src="https://example.invalid/frame"></iframe>',
  '',
  '<div onclick="window.__ran = 3" style="position:fixed">block html</div>',
  '',
  '[run it](javascript:window.__ran=4) [relative](./synthetic/relative/path) [anchor](#top)',
  '[html data](data:text/html;base64,PHNjcmlwdD4xPC9zY3JpcHQ+) [svg data](data:image/svg+xml,%3Csvg%20onload%3D1%3E)',
  '[vbscript](vbscript:msgbox) [file](file:///synthetic/secret) [ref link][ref]',
  '',
  'Bare: https://example.invalid/bare and www.example.invalid and <https://example.invalid/auto> and someone@example.invalid',
  '',
  '![a synthetic chart](https://example.invalid/pixel.png "title") ![](data:image/png;base64,iVBORw0KGgo=) ![svg](data:image/svg+xml,%3Csvg%3E)',
  '',
  'A note.[^1]',
  '',
  '[^1]: The footnote.',
  '',
  '[ref]: https://example.invalid/ref',
].join('\n');

beforeEach(() => {
  delete (window as { __ran?: unknown }).__ran;
});

it('draws raw HTML, links, images and footnotes as inert text: no element, no URL, no id', () => {
  const fetchSpy = vi.fn();
  const originalFetch = globalThis.fetch;
  globalThis.fetch = fetchSpy as unknown as typeof fetch;
  const openSpy = vi.spyOn(XMLHttpRequest.prototype, 'open');
  const before = window.location.href;
  try {
    render(<SessionTranscript records={[textRecord('h1', 'assistant', HOSTILE)]} />);
    const answer = blockNode('h1:0').querySelector('[data-format="markdown"]') as HTMLElement;
    expect(answer).toBeTruthy();

    // No element that runs, loads, frames or navigates.
    expect(
      answer.querySelectorAll(
        'a, img, picture, source, script, iframe, frame, object, embed, svg, math, video, audio, link, style, form, button, input, textarea, select, base, meta',
      ),
    ).toHaveLength(0);
    // No attribute that carries a URL, a handler or an identity — on anything.
    for (const element of [answer, ...answer.querySelectorAll('*')]) {
      for (const { name } of element.attributes) {
        expect(name).not.toMatch(
          /^(?:on|href$|src$|srcset$|action$|formaction$|xlink:|poster$|background$|id$|name$)/,
        );
      }
      if (element.hasAttribute('style'))
        expect(element.getAttribute('style')).toMatch(/^(?:text-align: (?:left|right|center);)?$/);
    }

    // The markup is shown as the characters it is.
    expect(answer.textContent).toContain('<script>window.__ran = 1</script>');
    expect(answer.textContent).toContain('<iframe src="https://example.invalid/frame"></iframe>');
    expect(answer.textContent).toContain('onclick="window.__ran = 3"');

    // A link is its label and, where it adds something, its destination — as text.
    const links = [...answer.querySelectorAll('[data-md-link]')].map((node) => node.textContent);
    expect(links).toEqual(
      expect.arrayContaining([
        'run it (javascript:window.__ran=4)',
        'relative (./synthetic/relative/path)',
        'anchor (#top)',
        'html data (data:text/html;base64,PHNjcmlwdD4xPC9zY3JpcHQ+)',
        'svg data (data:image/svg+xml,%3Csvg%20onload%3D1%3E)',
        'vbscript (vbscript:msgbox)',
        'file (file:///synthetic/secret)',
        'ref link (https://example.invalid/ref)',
        'https://example.invalid/bare',
        'www.example.invalid',
        'https://example.invalid/auto',
        'someone@example.invalid',
      ]),
    );

    // An image is named, never loaded.
    expect([...answer.querySelectorAll('[data-md-image]')].map((node) => node.textContent)).toEqual(
      ['Image not shown: a synthetic chart', 'Image not shown', 'Image not shown: svg'],
    );

    // Pressing any of it goes nowhere.
    for (const node of answer.querySelectorAll('[data-md-link], [data-md-image]'))
      fireEvent.click(node);
    expect(window.location.href).toBe(before);
    expect((window as { __ran?: unknown }).__ran).toBeUndefined();
    expect(fetchSpy).not.toHaveBeenCalled();
    expect(openSpy).not.toHaveBeenCalled();
    // Nothing in the page as a whole was made into a link or a resource either.
    expect(transcript().querySelectorAll('[href], [src], a, img')).toHaveLength(0);
  } finally {
    globalThis.fetch = originalFetch;
    openSpy.mockRestore();
  }
});

it('prints a link’s destination unless its label is exactly that destination', () => {
  const text = [
    // The two regressions: a label that is only the END of the destination, and a label that
    // names a different site. Both must show where the link really goes.
    '[docs](https://example.invalid/docs)',
    '[example.com](https://not-example.com)',
    // Exact equality, and the two rewrites the parser itself makes.
    '[https://example.invalid/same](https://example.invalid/same)',
    'https://example.invalid/bare',
    '<https://example.invalid/angle>',
    'www.example.invalid',
    'someone@example.invalid',
    '<other@example.invalid>',
    // Near misses of those rewrites are not rewrites.
    '[www.example.invalid](https://www.example.invalid)',
    '[someone@example.invalid](mailto:someone@example.invalid?subject=x)',
    '[example.invalid](http://www.example.invalid)',
    // The rewrites apply only to the labels they are for: `http://` to a `www.` label, `mailto:`
    // to an email-shaped one. Any other label keeps its scheme and target.
    '[docs](http://docs)',
    '[readme](mailto:readme)',
    '[someone@localhost](mailto:someone@localhost)',
    // GFM's `www.` literal is case-insensitive, and so is the rewrite it gets.
    'WWW.example.invalid',
    // An authored in-page anchor keeps its destination; a generated footnote link does not.
    '[top](#top) and a note[^n].',
    '',
    '[^n]: The note.',
    // One paragraph (soft breaks), so the answer stays under the clamp and is drawn formatted.
  ].join('\n');
  render(<SessionTranscript records={[textRecord('ln', 'assistant', text)]} />);
  const links = [...blockNode('ln:0').querySelectorAll('[data-md-link]')].map(
    (node) => node.textContent,
  );

  expect(links).toEqual([
    'docs (https://example.invalid/docs)',
    'example.com (https://not-example.com)',
    'https://example.invalid/same',
    'https://example.invalid/bare',
    'https://example.invalid/angle',
    'www.example.invalid',
    'someone@example.invalid',
    'other@example.invalid',
    'www.example.invalid (https://www.example.invalid)',
    'someone@example.invalid (mailto:someone@example.invalid?subject=x)',
    'example.invalid (http://www.example.invalid)',
    'docs (http://docs)',
    'readme (mailto:readme)',
    'someone@localhost (mailto:someone@localhost)',
    'WWW.example.invalid',
    'top (#top)',
    '1',
    '↩',
  ]);
  // Still inert: text, never a link.
  expect(blockNode('ln:0').querySelectorAll('a, [href]')).toHaveLength(0);
});

/** Activation from the keyboard, as far as jsdom carries it — see TranscriptInteraction. */
function pressFocused(control: HTMLElement) {
  control.focus();
  expect(document.activeElement).toBe(control);
  fireEvent.click(document.activeElement as HTMLElement);
}

it('previews a long answer literally, parses it only when opened, and keeps every character', () => {
  const rows = Array.from({ length: 120 }, (_, row) => `| synthetic-${row + 1} | ${row * 3} |`);
  const fenceLines = Array.from({ length: 80 }, (_, line) => `line ${line + 1}`);
  const prose = Array.from({ length: 40 }, (_, n) => `Paragraph ${n + 1} with **weight**.`);
  const long = [
    '# Report',
    '',
    ...prose.flatMap((line) => [line, '']),
    '| name | value |',
    '| --- | --- |',
    ...rows,
    '',
    '```text',
    ...fenceLines,
    '```',
    '',
    'The last sentence.',
  ].join('\n');
  expect(long.split('\n').length).toBeGreaterThan(CLAMP_LINES);

  render(<SessionTranscript records={[textRecord('l1', 'assistant', long)]} />);
  const block = blockNode('l1:0') as HTMLElement;

  // Collapsed: a bounded literal preview, nothing parsed, and nothing focusable hidden.
  const box = block.querySelector('[data-clamped]') as HTMLElement;
  expect(box.getAttribute('data-clamped')).toBe(String(CLAMP_LINES));
  expect(block.querySelector('[data-format="markdown"]')).toBeNull();
  const preview = box.querySelector('[data-preview]')!;
  expect(preview.textContent).toBe(long.split('\n').slice(0, CLAMP_LINES).join('\n'));
  expect(box.querySelectorAll('[tabindex], button, a, pre, table')).toHaveLength(0);

  const toggle = within(block).getByRole('button', { name: 'Show the rest of this block' });
  expect(toggle.getAttribute('aria-controls')).toBe(box.id);
  pressFocused(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(document.activeElement).toBe(toggle);
  expect(block.querySelector('[data-clamped]')).toBeNull();

  // Expanded: the whole answer, formatted, with every row, line and paragraph in it.
  const answer = block.querySelector('[data-format="markdown"]')!;
  expect(answer.querySelector('h3')?.textContent).toBe('Report');
  expect(answer.querySelectorAll('p strong')).toHaveLength(40);
  const table = within(block).getByRole('region', { name: 'Table, scrollable' });
  expect(table.querySelectorAll('tbody tr')).toHaveLength(120);
  expect(table.querySelector('tbody tr:last-child td')?.textContent).toBe('synthetic-120');
  const code = within(block).getByRole('region', { name: 'text, scrollable code' });
  expect(code.textContent).toBe(fenceLines.join('\n'));
  expect(answer.textContent).toContain('The last sentence.');
  // Both long surfaces are keyboard stops, bounded by their own box rather than the clamp.
  expect(table.tabIndex).toBe(0);
  expect(code.tabIndex).toBe(0);
  expect(code.closest('[data-clamped]')).toBeNull();
  expect(table.closest('[data-clamped]')).toBeNull();

  pressFocused(toggle);
  expect(document.activeElement).toBe(toggle);
  expect(block.querySelector('[data-format="markdown"]')).toBeNull();
  expect(block.querySelector('[data-preview]')).toBeTruthy();
});

it('bounds the preview of one enormous line by characters', () => {
  const line = 'word '.repeat(CLAMP_CHARACTERS);
  expect(clampPreview(line)).toHaveLength(CLAMP_CHARACTERS);
  expect(clampPreview('short')).toBe('short');
  expect(clampPreview('a\n'.repeat(CLAMP_LINES + 5))).toBe(
    Array.from({ length: CLAMP_LINES }, () => 'a').join('\n'),
  );

  render(<SessionTranscript records={[textRecord('w1', 'assistant', line)]} />);
  expect(blockNode('w1:0').querySelector('[data-preview]')?.textContent).toHaveLength(
    CLAMP_CHARACTERS,
  );
});

it('keeps record and block identities, and the first and a repeated reveal, exactly as before', () => {
  const records: TranscriptRecord[] = [
    textRecord('mk:record one', 'assistant', FORMATTED),
    {
      id: 'mk:record two',
      role: 'assistant',
      blocks: [
        { kind: 'text', id: 'mk:two:0', text: '### Plan\n\n- [x] read\n- [ ] run' },
        {
          kind: 'tool_call',
          id: 'mk:two:1',
          name: 'Bash',
          callId: 'call-mk-1',
          input: { kind: 'text', text: 'synthetic-tool' },
          outcome: { state: 'ok', output: { kind: 'text', text: 'ok' } },
        },
        { kind: 'text', id: 'mk:two:2', text: 'Done: **all** [green](https://example.invalid).' },
      ],
    },
  ];
  const scrolled: Element[] = [];
  Element.prototype.scrollIntoView = function (this: Element) {
    scrolled.push(this);
  } as Element['scrollIntoView'];
  const onReveal = vi.fn<(token: number, outcome: RevealOutcome) => void>();
  try {
    const { rerender } = render(<SessionTranscript records={records} onReveal={onReveal} />);

    expect(
      [...transcript().querySelectorAll('[data-record-id]')].map((n) =>
        n.getAttribute('data-record-id'),
      ),
    ).toEqual(['mk:record one', 'mk:record two']);
    expect(
      [...transcript().querySelectorAll('[data-block-id]')].map((n) =>
        n.getAttribute('data-block-id'),
      ),
    ).toEqual(['mk:record one:0', 'mk:two:0', 'mk:two:1', 'mk:two:2']);
    // Formatting adds no identity of its own anywhere in the page.
    expect(transcript().querySelectorAll('[data-format="markdown"] [id]')).toHaveLength(0);
    expect(blockNode('mk:two:0').querySelectorAll('[data-task]')).toHaveLength(2);
    expect(blockNode('mk:two:0').textContent).toContain('[x] read');

    const outside = screen.getAllByRole('region', { name: /scrollable code/ })[0];
    outside.focus();

    act(() => {
      rerender(
        <SessionTranscript
          records={records}
          reveal={{ token: 1, blockId: 'mk:two:1' }}
          onReveal={onReveal}
        />,
      );
    });
    expect(onReveal).toHaveBeenLastCalledWith(1, 'revealed');
    const card = transcript().querySelector('[data-call-id="call-mk-1"]')!;
    expect(card.getAttribute('data-block-id')).toBe('mk:two:1');
    expect(card.hasAttribute('data-highlighted')).toBe(true);
    expect(scrolled.at(-1)).toBe(card);
    expect(document.activeElement).toBe(outside);

    // The same call again, under a new token: answered again, on the same card.
    act(() => {
      rerender(
        <SessionTranscript
          records={records}
          reveal={{ token: 2, blockId: 'mk:two:1' }}
          onReveal={onReveal}
        />,
      );
    });
    expect(onReveal).toHaveBeenLastCalledWith(2, 'revealed');
    expect(onReveal).toHaveBeenCalledTimes(2);
    expect(transcript().querySelector('[data-call-id="call-mk-1"]')).toBe(card);
    expect(card.hasAttribute('data-highlighted')).toBe(true);
    expect(document.activeElement).toBe(outside);
  } finally {
    delete (Element.prototype as Partial<Element>).scrollIntoView;
  }
});
