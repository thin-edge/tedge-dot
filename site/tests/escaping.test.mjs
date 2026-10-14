import { describe, expect, it } from 'vitest';
import rehypeRaw from 'rehype-raw';
import rehypeStringify from 'rehype-stringify';
import remarkGfm from 'remark-gfm';
import remarkParse from 'remark-parse';
import remarkRehype from 'remark-rehype';
import { unified } from 'unified';
import { cell } from '../scripts/lib/schema-reference.mjs';
import { stripComments } from '../scripts/lib/whats-new.mjs';

// The cells of a one-row table, rendered the way the site renders Markdown.
async function renderCells(...cells) {
  const md = `| ${cells.map((_, i) => `h${i}`).join(' | ')} |\n| ${cells.map(() => '---').join(' | ')} |\n| ${cells.join(' | ')} |\n`;
  const html = String(
    await unified().use(remarkParse).use(remarkGfm).use(remarkRehype, { allowDangerousHtml: true }).use(rehypeRaw).use(rehypeStringify).process(md),
  );
  return [...html.matchAll(/<td>([\s\S]*?)<\/td>/g)].map((m) => m[1]);
}

describe('schema reference table cells', () => {
  it('keeps backslashes and pipes in text, in the same cell', async () => {
    expect(await renderCells(cell('a\\|b or c|d, path C:\\dir'), 'next')).toEqual(['a\\|b or c|d, path C:\\dir', 'next']);
  });

  it('keeps pipes and backslashes inside code spans', async () => {
    expect(await renderCells(cell('use `a|b` or `x\\|y`'), 'next')).toEqual(['use <code>a|b</code> or <code>x\\|y</code>', 'next']);
  });

  it('leaves plain code spans as Markdown, and escapes < in text', () => {
    expect(cell('`<plain>` and a<b')).toBe('`<plain>` and a&lt;b');
  });
});

describe("What's new comment stripping", () => {
  it('drops comments, also ones that a first pass would join together', () => {
    expect(stripComments('a<!-- x -->b')).toBe('ab');
    expect(stripComments('a<!<!-- x -->-- y -->b')).toBe('ab');
  });

  it('shows an unclosed comment as text instead of hiding the rest', () => {
    expect(stripComments('a <!-- never closed')).toBe('a &lt;!-- never closed');
  });
});
