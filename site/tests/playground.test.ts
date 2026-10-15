import { describe, expect, it } from 'vitest';
import { changed, policyFromTable } from '../src/lib/report';
import { simulate, tableToToml } from '../src/lib/playground';

describe('playground simulation', () => {
  it('publishes every reading without a policy', () => {
    const run = simulate('noisy', {});
    expect(run.published.length).toBe(run.readings.length);
  });

  it('marks only readings that differ from the last published by the deadband (spec scenario)', () => {
    const run = simulate('noisy', { deadband: 0.5 });
    const policy = policyFromTable({ deadband: 0.5 });
    let last: number | null = null;
    for (const p of run.published) {
      const v = run.readings[p.index].value;
      if (last !== null) expect(changed(policy, { kind: 'number', value: v }, { kind: 'number', value: last })).toBe(true);
      last = v;
    }
    expect(run.published.length).toBeLessThan(run.readings.length / 4);
  });

  it('publishes a flat signal only on its heartbeat', () => {
    const run = simulate('flat', { on_change: true, max_interval: '2m' });
    expect(run.published.length).toBe(1 + 5);
    expect(run.published.filter((p) => p.heartbeat).length).toBe(5);
  });

  it('renders the table as TOML', () => {
    expect(tableToToml({ on_change: true, deadband: '2%', max_interval: '30m' })).toBe(
      'report = { on_change = true, deadband = "2%", max_interval = "30m" }',
    );
    expect(tableToToml({})).toBe('report = {}');
  });
});
