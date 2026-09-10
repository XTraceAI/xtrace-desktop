import { resolve, dirname, sep } from 'node:path';

const ui = resolve(import.meta.dirname, '..');
const gallery = resolve(ui, 'src/gallery');
const entry = resolve(ui, 'src/app/AppRoutes.tsx');
const literal = (node) => {
  if (node?.type === 'Literal' && typeof node.value === 'string') return node.value;
  if (node?.type === 'TemplateLiteral')
    return node.quasis.map((part) => part.value.cooked).join('*');
  if (node?.type === 'BinaryExpression' && node.operator === '+') {
    const left = literal(node.left),
      right = literal(node.right);
    if (left !== undefined && right !== undefined) return left + right;
  }
};
const inside = (path, root) => path === root || path.startsWith(root + sep);
export const galleryBoundary = {
  meta: {
    type: 'problem',
    schema: [],
    messages: {
      boundary:
        'Gallery modules are illustrative development inputs and cannot be imported into product code.',
    },
  },
  create(context) {
    const filename = resolve(context.filename);
    if (inside(filename, gallery)) return {};
    const text = (node) => context.sourceCode.getText(node).replace(/\s/g, '');
    function guarded(node) {
      let child = node;
      for (let parent = node.parent; parent; child = parent, parent = parent.parent) {
        if (parent.type === 'ConditionalExpression' && parent.consequent === child) {
          return /^import\.meta\.env\.DEV&&import\.meta\.env\.VITE_GALLERY===["']1["']$/.test(
            text(parent.test),
          );
        }
      }
      return false;
    }
    function check(node, source) {
      const value = literal(source);
      const target = value?.startsWith('.') ? resolve(dirname(filename), value) : value;
      const wildcard = value?.search(/[*?{[]/);
      const prefix =
        wildcard !== undefined && wildcard >= 0
          ? resolve(dirname(filename), dirname(value.slice(0, wildcard) + '_'))
          : undefined;
      const referencesGallery =
        target &&
        (inside(target, gallery) ||
          /(^|\/)gallery(\/|$)/.test(target) ||
          (prefix && inside(gallery, prefix)));
      if (!referencesGallery) return;
      const allowed =
        filename === entry &&
        node.type === 'ImportExpression' &&
        target === resolve(gallery, 'Gallery') &&
        guarded(node);
      if (!allowed) context.report({ node, messageId: 'boundary' });
    }
    return {
      ImportDeclaration: (node) => check(node, node.source),
      ExportNamedDeclaration: (node) => {
        if (node.source) check(node, node.source);
      },
      ExportAllDeclaration: (node) => check(node, node.source),
      ImportExpression: (node) => check(node, node.source),
      CallExpression: (node) => {
        if (node.callee.type === 'Identifier' && node.callee.name === 'require')
          check(node, node.arguments[0]);
        if (text(node.callee) === 'import.meta.glob') {
          const argument = node.arguments[0];
          for (const source of argument?.type === 'ArrayExpression'
            ? argument.elements
            : [argument])
            if (source) check(node, source);
        }
      },
    };
  },
};
