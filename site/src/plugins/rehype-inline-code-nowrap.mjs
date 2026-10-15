// Short inline code with a hyphen or a space stays on one line. Browsers break inline code at
// those, which left a lone "-" chip at a line end for `-p`, `--pki-dir` or `tedge-dot-rs`; a short
// span is cheaper to move to the next line whole. Anything longer, or without a break point of
// its own (`trust_any_server_certificate`), keeps the normal wrapping, so it never widens a
// phone layout: NOWRAP_MAX characters of code fit any line.
import { visit } from 'unist-util-visit';

export const NOWRAP_MAX = 24;

export default function rehypeInlineCodeNowrap() {
  return (tree) => {
    visit(tree, 'element', (node, _index, parent) => {
      if (node.tagName !== 'code' || parent?.tagName === 'pre') return;
      const text = toText(node);
      if (text.length > NOWRAP_MAX || !/[- ]/.test(text)) return;
      const cls = node.properties.className ?? [];
      node.properties.className = [...(Array.isArray(cls) ? cls : [cls]), 'td-nowrap'];
    });
  };
}

function toText(node) {
  if (node.type === 'text') return node.value;
  return (node.children ?? []).map(toText).join('');
}
