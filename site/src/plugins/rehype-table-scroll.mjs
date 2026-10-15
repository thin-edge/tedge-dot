// Every table gets a scroll wrapper. The table itself fills the column and wraps its cells;
// only a table whose words are still wider than the column scrolls, inside the wrapper, which
// shows a shadow at the edge that has more. The page never scrolls sideways.
//
// A table of three or more columns also gets each cell labelled with its column's header
// (data-label), so that on a phone the theme can show every row as a stacked card
// ("Default: 30s") instead of squeezing four columns into 360px.
import { visit, SKIP } from 'unist-util-visit';

export default function rehypeTableScroll() {
  return (tree) => {
    visit(tree, 'element', (node, index, parent) => {
      if (node.tagName !== 'table' || !parent || index === undefined) return;
      if (parent.properties?.className?.includes?.('td-table-scroll')) return;
      const headers = headerTexts(node);
      const stack = headers.length >= 3;
      if (stack) labelCells(node, headers);
      parent.children[index] = {
        type: 'element',
        tagName: 'div',
        properties: { className: stack ? ['td-table-scroll', 'td-stack'] : ['td-table-scroll'], tabIndex: 0, role: 'region', ariaLabel: 'Table' },
        children: [node],
      };
      return [SKIP, index + 1];
    });
  };
}

const elements = (node, tag) => (node.children ?? []).filter((c) => c.type === 'element' && (!tag || c.tagName === tag));
const text = (node) => (node.type === 'text' ? node.value : (node.children ?? []).map(text).join(''));

function headerTexts(table) {
  const thead = elements(table, 'thead')[0];
  const row = thead && elements(thead, 'tr')[0];
  return row ? elements(row).map((cell) => text(cell).trim()) : [];
}

function labelCells(table, headers) {
  for (const body of elements(table, 'tbody')) {
    for (const row of elements(body, 'tr')) {
      elements(row).forEach((cell, i) => {
        if (headers[i]) cell.properties = { ...cell.properties, dataLabel: headers[i] };
      });
    }
  }
}
