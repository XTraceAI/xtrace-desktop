import { type ElementType, memo, type ReactNode } from 'react';
import ReactMarkdown, { type Components, type ExtraProps } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { CodeSurface } from './TranscriptText';
import './transcript.css';

/**
 * The agent's answer, drawn as the Markdown it was written in.
 *
 * **Which text this is for is the caller's decision, and it is narrow.** An agent's text block
 * IS Markdown: it is what the model emitted, and `**unbounded**` in it is emphasis the model
 * asked for. A person's prompt is not — it was typed into a plain composer, and a stray
 * backtick in it is a character they typed. Thinking, tool inputs and results, and the record's
 * own code blocks stay literal for the same reason. This is the presentation contract the
 * memory-hub transcript already uses, kept so the two read a session the same way.
 *
 * What formatting may do is draw text differently. It may not make the record do anything:
 *
 * - **No raw HTML.** There is no `rehype-raw`, so `<script>` in an answer arrives as a raw node
 *   and `react-markdown` turns it back into a text node — its characters are shown, nothing is
 *   parsed into an element and nothing runs.
 * - **No links.** A Markdown link, and the bare URL GFM would auto-link, render as inert text:
 *   the label, then the destination in parentheses when it says something the label does not.
 *   Nothing is an `<a>`, nothing carries an `href`, and nothing navigates — whatever the scheme.
 * - **No images.** An image is a sentence naming it, not an `<img>`: this view fetches nothing.
 * - **No ids.** Every element is drawn from a fixed list with only the props this file gives
 *   it, so GFM footnotes put no `id` into the page and nothing here can collide with a record's
 *   or block's own identity.
 * - **No other element.** Anything outside {@link ALLOWED} is unwrapped to its text, not
 *   dropped, so an unexpected construct loses its formatting rather than its words.
 *
 * Fences go to the transcript's existing {@link CodeSurface}: bounded, scrolling, a named
 * keyboard stop, and literal. A wide or long table scrolls inside its own named, focusable box
 * for the same reason — so a reader who cannot use a pointer can still reach its last row.
 *
 * No math, no highlighting and no streaming parser: a transcript is a finished record, and
 * `$5 and $10` in an answer is money.
 */
export const TranscriptMarkdown = memo(function TranscriptMarkdown({ text }: { text: string }) {
  // Memoised on `text` alone, so a transcript re-rendering for a reveal or a sibling's toggle
  // does not parse an unchanged answer again. Nothing is cached beyond the mounted component:
  // an answer that is not on screen holds no parse anywhere.
  return (
    <div className="xt-md" data-format="markdown">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        components={COMPONENTS}
        allowedElements={ALLOWED}
        unwrapDisallowed
        urlTransform={keepUrlAsText}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
});

const REMARK_PLUGINS = [remarkGfm];

/**
 * Leave every destination as written.
 *
 * Safe only because of what the components below do with it: no element this file draws takes
 * a URL attribute, so a destination is only ever printed as characters. Printing it verbatim is
 * the honest choice — a sanitiser would blank `javascript:…` and leave the reader wondering what
 * the link said.
 */
function keepUrlAsText(url: string): string {
  return url;
}

/** Every element a rendered answer may contain. Anything else is unwrapped to its children. */
const ALLOWED = [
  'p',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'ul',
  'ol',
  'li',
  'blockquote',
  'em',
  'strong',
  'del',
  'hr',
  'br',
  'code',
  'pre',
  'table',
  'thead',
  'tbody',
  'tr',
  'th',
  'td',
  'input',
  'sup',
  'section',
  'a',
  'img',
];

type HastLike = { type?: string; value?: unknown; tagName?: string; children?: unknown[] };

/** The characters under a hast node, with no formatting. */
function textOf(node: unknown): string {
  if (!node || typeof node !== 'object') return '';
  const current = node as HastLike;
  if (current.type === 'text' && typeof current.value === 'string') return current.value;
  return Array.isArray(current.children) ? current.children.map(textOf).join('') : '';
}

/** The language a fence named, from the `language-…` class the parser put on its `<code>`. */
function fenceLanguage(pre: ExtraProps['node']): string | null {
  const code = pre?.children.find(
    (child) => child.type === 'element' && child.tagName === 'code',
  ) as { properties?: { className?: unknown } } | undefined;
  const classes = code?.properties?.className;
  if (!Array.isArray(classes)) return null;
  for (const name of classes) {
    if (typeof name === 'string' && name.startsWith('language-')) return name.slice(9) || null;
  }
  return null;
}

/**
 * A heading, moved under the page's own.
 *
 * The page is an `h1` and its sections are `h2`, so an answer's `# Title` becomes an `h3`: a
 * reader moving by heading meets the answer's structure inside the transcript, not a second
 * page title in the middle of it. The original level is kept for styling.
 */
function heading(level: 1 | 2 | 3 | 4 | 5 | 6) {
  const Tag = `h${Math.min(6, level + 2)}` as 'h3' | 'h4' | 'h5' | 'h6';
  return function Heading({ children }: { children?: ReactNode }) {
    return (
      <Tag className="xt-md-heading" data-level={level}>
        {children}
      </Tag>
    );
  };
}

/** An element drawn with its children and nothing else — no id, no class, no attribute. */
function plain(Tag: ElementType) {
  return function Plain({ children }: { children?: ReactNode }) {
    return <Tag>{children}</Tag>;
  };
}

/**
 * Whether a link's label already says its whole destination, so printing it again adds nothing.
 *
 * Exact, and deliberately narrow: the label must BE the destination, or be it under one of the
 * two rewrites the parser itself makes — and only for the labels that rewrite applies to. GFM
 * turns a `www.…` literal into `http://www.…`, and an email address (bare or `<…>`) into
 * `mailto:…`. Anything looser lies: a suffix match would hide the destination of
 * `[docs](https://example.invalid/docs)` and present `[example.com](https://not-example.com)`
 * as going to the site its label names, and prefixing ANY label would hide the scheme of
 * `[docs](http://docs)` and the target of `[readme](mailto:readme)`. When in doubt the
 * destination is printed.
 */
function labelIsDestination(label: string, target: string): boolean {
  if (label === '') return false;
  if (target === label) return true;
  if (target === `http://${label}`) return WWW_LABEL.test(label);
  if (target === `mailto:${label}`) return EMAIL_LABEL.test(label);
  return false;
}

/** A label GFM would have auto-linked as a `www.` literal. */
const WWW_LABEL = /^www\./i;

/**
 * A label shaped like the email addresses GFM auto-links: a local part of letters, digits and
 * `.+-_`, `@`, and two or more dot-separated domain labels, each ending in a letter or digit.
 * Never wider than the parser's own rule, so a label it misses only has its destination printed.
 */
const EMAIL_LABEL = /^[A-Za-z0-9._+-]+@(?:[A-Za-z0-9_-]*[A-Za-z0-9]\.)+[A-Za-z0-9_-]*[A-Za-z0-9]$/;

/**
 * A link GFM generated for a footnote — the `[^1]` reference or its `↩` back-reference.
 *
 * Its destination is an in-page anchor the parser composed, not one the answer wrote, and this
 * view renders no ids for it to point at. Recognised by the markers the parser sets on exactly
 * those two links; an authored `[text](#anchor)` carries neither and keeps its destination.
 */
function generatedFootnoteLink(node: ExtraProps['node']): boolean {
  const properties = node?.properties;
  return properties?.dataFootnoteRef !== undefined || properties?.dataFootnoteBackref !== undefined;
}

type Align = 'left' | 'right' | 'center';
function cellAlign(style: unknown): Align | undefined {
  const value = (style as { textAlign?: unknown } | undefined)?.textAlign;
  return value === 'left' || value === 'right' || value === 'center' ? value : undefined;
}

const COMPONENTS: Components = {
  p: plain('p'),
  h1: heading(1),
  h2: heading(2),
  h3: heading(3),
  h4: heading(4),
  h5: heading(5),
  h6: heading(6),
  ul: plain('ul'),
  ol: ({ start, children }) => (
    <ol start={typeof start === 'number' && start !== 1 ? start : undefined}>{children}</ol>
  ),
  li: ({ className, children }) => (
    <li data-task={className?.includes('task-list-item') ? '' : undefined}>{children}</li>
  ),
  blockquote: plain('blockquote'),
  em: plain('em'),
  strong: plain('strong'),
  del: plain('del'),
  hr: () => <hr />,
  br: () => <br />,
  sup: plain('sup'),
  section: plain('section'),

  // Only inline code reaches here: a fence's `<code>` is inside a `<pre>`, which is drawn whole
  // below and never renders its children.
  code: ({ children }) => <code className="xt-md-code">{children}</code>,
  pre: ({ node }) => (
    // The parser ends every fence with a newline it added; the fence did not contain it.
    <CodeSurface code={textOf(node).replace(/\n$/, '')} label={fenceLanguage(node)} />
  ),

  table: ({ children }) => (
    <div className="xt-md-table" tabIndex={0} role="region" aria-label="Table, scrollable">
      <table>{children}</table>
    </div>
  ),
  thead: plain('thead'),
  tbody: plain('tbody'),
  tr: plain('tr'),
  th: ({ style, children }) => <th style={{ textAlign: cellAlign(style) }}>{children}</th>,
  td: ({ style, children }) => <td style={{ textAlign: cellAlign(style) }}>{children}</td>,

  // A task-list box states its state; it is not a control. A disabled `<input>` would still be
  // announced as a checkbox the reader cannot operate, so it is words instead.
  input: ({ checked }) => (
    <span className="xt-md-task" data-checked={checked ? '' : undefined}>
      {checked ? '[x]' : '[ ]'}
    </span>
  ),

  a: ({ node, href, children }) => {
    const target = typeof href === 'string' ? href : '';
    const shown =
      !target || generatedFootnoteLink(node) || labelIsDestination(textOf(node), target);
    return (
      <span className="xt-md-link" data-md-link="">
        {children}
        {shown ? null : <span className="xt-md-link-target"> ({target})</span>}
      </span>
    );
  },
  img: ({ alt }) => (
    <span className="xt-md-image" data-md-image="">
      {alt?.trim() ? `Image not shown: ${alt.trim()}` : 'Image not shown'}
    </span>
  ),
};
