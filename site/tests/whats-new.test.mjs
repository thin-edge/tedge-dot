import { describe, expect, it } from 'vitest';
import { groupByRelease, renderWhatsNew } from '../scripts/lib/whats-new.mjs';

const tags = ['0.0.14', '0.0.15', '0.0.16', '0.0.17'];
const published = (t) => t !== '0.0.15';
const frag = (file, containedIn) => ({ file, content: `### ${file}\n\nText.\n`, containedIn });

describe("What's new grouping", () => {
  it('puts a highlight under the first release that contained it (spec scenario)', () => {
    const { releases } = groupByRelease([frag('changes/foo.md', ['0.0.17'])], tags, published);
    expect(releases[0].tag).toBe('0.0.17');
    expect(releases[0].highlights.map((f) => f.file)).toEqual(['changes/foo.md']);
  });

  it('carries the highlights of an unpublished tag to the next published release', () => {
    const { releases } = groupByRelease([frag('changes/bar.md', ['0.0.15', '0.0.16', '0.0.17'])], tags, published);
    const r16 = releases.find((r) => r.tag === '0.0.16');
    expect(r16.highlights.map((f) => f.file)).toEqual(['changes/bar.md']);
    expect(r16.carries).toEqual(['0.0.15']);
    expect(releases.map((r) => r.tag)).not.toContain('0.0.15');
  });

  it('lists a highlight no release contains under Unreleased, and releases newest first', () => {
    const grouped = groupByRelease([frag('changes/new.md', [])], tags, published);
    expect(grouped.unreleased.map((f) => f.file)).toEqual(['changes/new.md']);
    expect(grouped.releases.map((r) => r.tag)).toEqual(['0.0.17', '0.0.16', '0.0.14']);
    const page = renderWhatsNew(grouped, { repository: 'thin-edge/tedge-dot' });
    expect(page.indexOf('## Unreleased')).toBeLessThan(page.indexOf('## 0.0.17'));
    expect(page).toContain('https://github.com/thin-edge/tedge-dot/releases/tag/0.0.16');
    expect(page).toContain('This release also delivers 0.0.15');
  });

  it('orders versions numerically, not as text', () => {
    const { releases } = groupByRelease([], ['0.0.9', '0.0.10', '0.0.2'], () => true);
    expect(releases.map((r) => r.tag)).toEqual(['0.0.10', '0.0.9', '0.0.2']);
  });
});
