// Simulation behind the report-policy playground: poll a synthetic signal and run every
// reading through the SDK's reporting policy (src/lib/report.ts).

import { changed, policyFromTable, ReportState, type Obs, type ReportTable } from './report';

export type SignalId = 'sine' | 'step' | 'noisy' | 'flat' | 'flapping';

export const SIGNALS: { id: SignalId; label: string; unit: string }[] = [
  { id: 'noisy', label: 'Noisy temperature', unit: '°C' },
  { id: 'sine', label: 'Slow sine', unit: '°C' },
  { id: 'step', label: 'Setpoint steps', unit: '°C' },
  { id: 'flat', label: 'Flat (idle sensor)', unit: '°C' },
  { id: 'flapping', label: 'Flapping switch', unit: '' },
];

/** A small deterministic PRNG, so a preset always draws the same signal. */
function mulberry32(seed: number) {
  return () => {
    seed |= 0;
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** The signal's value at `t` ms, for a run of `duration` ms. */
export function signalAt(id: SignalId, t: number, duration: number, rand: () => number): number {
  const s = t / 1000;
  const round = (v: number, places = 2) => Math.round(v * 10 ** places) / 10 ** places;
  switch (id) {
    case 'sine':
      return round(21 + 4 * Math.sin((2 * Math.PI * s) / 240));
    case 'step': {
      const f = t / duration;
      return f < 0.3 ? 20 : f < 0.65 ? 24 : 22;
    }
    case 'noisy': {
      // Gaussian-ish noise (sum of uniforms) on a slow drift.
      const noise = (rand() + rand() + rand() - 1.5) * 0.4;
      return round(20 + s / 200 + noise);
    }
    case 'flat':
      return 21.5;
    case 'flapping': {
      // A switch that turns on twice, bouncing for a few seconds each time.
      const edges = [0.25, 0.45, 0.65, 0.85].map((f) => f * duration);
      const state = edges.filter((e) => t >= e).length % 2;
      const near = edges.some((e) => t >= e && t - e < 4000);
      return near && rand() < 0.5 ? 1 - state : state;
    }
  }
}

export interface Reading {
  t: number;
  value: number;
}

export interface Published {
  /** The reading's index (and so its timestamp). */
  index: number;
  /** When it was published: later than the reading when it was held back. */
  at: number;
  /** Published although unchanged, because the heartbeat (`max_interval`) was due. */
  heartbeat: boolean;
}

export interface Run {
  readings: Reading[];
  published: Published[];
}

/** Poll the signal every `pollMs` for `durationMs`, through the policy. */
export function simulate(
  signal: SignalId,
  table: ReportTable,
  { durationMs = 600_000, pollMs = 1000, tickMs = 100, seed = 7 } = {},
): Run {
  const rand = mulberry32(seed);
  const policy = policyFromTable(table);
  const state = new ReportState<number>(policy, 0);
  const readings: Reading[] = [];
  const published: Published[] = [];
  let last: { obs: Obs; at: number } | null = null;

  const record = (index: number, at: number, viaOffer: boolean) => {
    const obs: Obs = { kind: 'number', value: readings[index].value };
    const heartbeat =
      viaOffer &&
      last !== null &&
      policy.maxInterval !== null &&
      at - last.at >= policy.maxInterval &&
      !changed(policy, obs, last.obs);
    published.push({ index, at, heartbeat });
    last = { obs, at };
  };

  for (let t = 0; t <= durationMs; t += tickMs) {
    if (t % pollMs === 0) {
      const value = signalAt(signal, t, durationMs, rand);
      readings.push({ t, value });
      const index = readings.length - 1;
      const out = state.offer(index, { kind: 'number', value }, 'good', t);
      if (out !== null) record(out, t, true);
    }
    const due = state.due(t, false);
    if (due.publish !== null) record(due.publish, t, false);
  }
  return { readings, published };
}

/** The `report` table as a line of TOML, with only the keys that are set. */
export function tableToToml(table: ReportTable): string {
  const parts: string[] = [];
  if (table.on_change) parts.push('on_change = true');
  if (table.deadband !== undefined && table.deadband !== '') {
    parts.push(`deadband = ${typeof table.deadband === 'number' ? table.deadband : JSON.stringify(table.deadband)}`);
  }
  for (const key of ['min_interval', 'max_interval', 'debounce'] as const) {
    if (table[key]) parts.push(`${key} = ${JSON.stringify(table[key])}`);
  }
  return `report = { ${parts.join(', ')} }`.replace('{  }', '{}');
}
