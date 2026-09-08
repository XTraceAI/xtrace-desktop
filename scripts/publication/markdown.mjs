import { randomUUID } from 'node:crypto';
import { HtmlRenderer, Node, Parser } from 'commonmark';
import { parseFragment } from 'parse5';

export const ATTESTATION =
  'I reviewed the final PR text, linked issues, comments, and attachments for public disclosure.';

// CommonMark identifies real Markdown controls; HTML parsing then ensures an
// unclosed container cannot hide a control that appears top-level in the AST.
export function disclosureControls(body) {
  const tree = new Parser().parse(body);
  const lines = body.split(/\r?\n/);
  const candidates = [];
  const walker = tree.walker();
  let event;
  while ((event = walker.next())) {
    const node = event.node;
    if (!event.entering || node.type !== 'paragraph') continue;
    const lineNumber = node.sourcepos[0][0] - 1;
    const line = lines[lineNumber];
    const match = /^ {0,3}- \[([ xX])\] (.+?)\s*$/.exec(line);
    const declaration = match?.[2] === ATTESTATION ? match : null;
    const snapshot = /^Disclosure snapshot: (pending|[a-f0-9]{64})\s*$/.exec(line);
    const listItem = node.parent?.type === 'item' && node.parent.parent?.parent === tree;
    if ((!declaration || !listItem) && (!snapshot || node.parent !== tree)) continue;
    if (node.sourcepos[1][0] !== lineNumber + 1) continue;
    const marker = 'publication-control-' + randomUUID();
    candidates.push({ marker, lineNumber, declaration, snapshot, node });
  }
  for (const { node, marker } of candidates) {
    while (node.firstChild) node.firstChild.unlink();
    const text = new Node('text');
    text.literal = marker;
    node.appendChild(text);
  }
  const document = parseFragment(new HtmlRenderer().render(tree));
  const visible = new Set();
  const allowed = new Set(['#document-fragment', 'ul', 'li', 'p']);
  const visit = (node, permitted) => {
    if (node.nodeName === '#text') {
      if (permitted) visible.add(node.value);
      return;
    }
    for (const child of node.childNodes ?? [])
      visit(child, permitted && allowed.has(node.nodeName));
  };
  visit(document, true);
  return candidates.filter((item) => visible.has(item.marker));
}

export function referenceProse(body) {
  const document = parseFragment(new HtmlRenderer().render(new Parser().parse(body)));
  const parts = [];
  const excluded = new Set(['code', 'pre', 'script', 'style', 'textarea']);
  const visit = (node) => {
    if (excluded.has(node.nodeName)) return;
    if (node.nodeName === '#text') {
      parts.push(node.value);
      return;
    }
    if (node.nodeName === 'a') {
      const href = node.attrs.find((attribute) => attribute.name === 'href')?.value;
      if (href) {
        try {
          const url = new URL(href, 'https://github.com/');
          if (
            ['http:', 'https:'].includes(url.protocol) &&
            url.hostname === 'github.com' &&
            !url.port
          )
            parts.push(' https://github.com' + decodeURI(url.pathname) + ' ');
        } catch {
          // An invalid destination cannot identify a GitHub issue.
        }
      }
    }
    const block = /^(?:p|div|li|br|tr|td|th|h[1-6]|blockquote)$/.test(node.nodeName);
    if (block) parts.push('\n');
    for (const child of node.childNodes ?? []) visit(child);
    if (block) parts.push('\n');
  };
  // HTML comments have no text children; code/raw-text containers are skipped.
  // Rendering first preserves the context of inline HTML tags across AST nodes.
  visit(document);
  return parts.join('');
}
