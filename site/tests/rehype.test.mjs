import { describe, expect, it } from 'vitest';
import rehypeInlineCodeNowrap from '../src/plugins/rehype-inline-code-nowrap.mjs';

const tree = (children) => ({ type: 'root', children });
const el = (tagName, children, properties = {}) => ({ type: 'element', tagName, properties, children });
const text = (value) => ({ type: 'text', value });

describe('rehype-inline-code-nowrap', () => {
  it('marks short inline code, not long code or code in a block', () => {
    const short = el('code', [text('--pki-dir')]);
    const plain = el('code', [text('trust_any_server_certificate')]);
    const long = el('code', [text('cargo build --manifest-path impl/rust/Cargo.toml')]);
    const block = el('code', [text('-p')]);
    rehypeInlineCodeNowrap()(tree([el('p', [short, long, plain]), el('pre', [block])]));
    expect(short.properties.className).toEqual(['td-nowrap']);
    expect(long.properties.className).toBeUndefined();
    expect(plain.properties.className).toBeUndefined();
    expect(block.properties.className).toBeUndefined();
  });
});

describe('rehype-table-scroll', async () => {
  const { default: rehypeTableScroll } = await import('../src/plugins/rehype-table-scroll.mjs');
  it('labels the cells of a table of three or more columns for the stacked phone layout', () => {
    const row = (tag, cells) => el('tr', cells.map((c) => el(tag, [text(c)])));
    const table = el('table', [el('thead', [row('th', ['Key', 'Type', 'Default'])]), el('tbody', [row('td', ['port', 'integer', '1883'])])]);
    const root = tree([table]);
    rehypeTableScroll()(root);
    expect(root.children[0].properties.className).toEqual(['td-table-scroll', 'td-stack']);
    const cells = table.children[1].children[0].children;
    expect(cells.map((c) => c.properties.dataLabel)).toEqual(['Key', 'Type', 'Default']);
  });

  it('wraps each table once in a focusable scroll region', () => {
    const table = el('table', []);
    const root = tree([table]);
    rehypeTableScroll()(root);
    rehypeTableScroll()(root);
    expect(root.children).toHaveLength(1);
    expect(root.children[0].properties.className).toEqual(['td-table-scroll']);
    expect(root.children[0].children[0]).toBe(table);
  });
});
