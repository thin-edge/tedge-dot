/** @jsxImportSource preact */
// The report-policy playground: pick a signal, set a `report` table, and see which readings
// the SDK's policy would publish. The engine is src/lib/report.ts, which runs the contract's
// test vectors in CI.

import { useMemo, useRef, useState } from 'preact/hooks';
import { checkTable, type ReportTable } from '../../lib/report';
import { SIGNALS, simulate, tableToToml, type SignalId } from '../../lib/playground';
import './report-playground.css';

interface Form {
  on_change: boolean;
  deadband: string;
  min_interval: string;
  max_interval: string;
  debounce: string;
}

const PRESETS: { label: string; signal: SignalId; form: Form }[] = [
  { label: 'Deadband on a noisy signal', signal: 'noisy', form: { on_change: false, deadband: '0.5', min_interval: '', max_interval: '', debounce: '' } },
  { label: 'Heartbeat on a flat signal', signal: 'flat', form: { on_change: true, deadband: '', min_interval: '', max_interval: '2m', debounce: '' } },
  { label: 'Rate limit a sine', signal: 'sine', form: { on_change: true, deadband: '', min_interval: '30s', max_interval: '', debounce: '' } },
  { label: 'Debounce a flapping switch', signal: 'flapping', form: { on_change: true, deadband: '', min_interval: '', max_interval: '', debounce: '5s' } },
  { label: 'Percent deadband + heartbeat', signal: 'step', form: { on_change: false, deadband: '2%', min_interval: '', max_interval: '3m', debounce: '' } },
];

function toTable(form: Form): Record<string, unknown> {
  const table: Record<string, unknown> = {};
  if (form.on_change) table.on_change = true;
  const db = form.deadband.trim();
  if (db) table.deadband = db.endsWith('%') ? db : /^-?[0-9.]+$/.test(db) ? Number(db) : db;
  for (const key of ['min_interval', 'max_interval', 'debounce'] as const) {
    if (form[key].trim()) table[key] = form[key].trim();
  }
  return table;
}

const W = 760;
const H = 260;
const PAD = { top: 16, right: 16, bottom: 28, left: 44 };

export default function ReportPlayground() {
  const [signal, setSignal] = useState<SignalId>(PRESETS[0].signal);
  const [form, setForm] = useState<Form>(PRESETS[0].form);
  const [hover, setHover] = useState<number | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  const table = toTable(form);
  const error = checkTable(table);
  const run = useMemo(
    () => (error ? null : simulate(signal, table as ReportTable)),
    [signal, JSON.stringify(table), error],
  );
  const unit = SIGNALS.find((s) => s.id === signal)!.unit;

  const set = (key: keyof Form) => (e: Event) => {
    const t = e.currentTarget as HTMLInputElement;
    setForm({ ...form, [key]: t.type === 'checkbox' ? t.checked : t.value });
  };

  // Scales
  const values = run?.readings.map((r) => r.value) ?? [0, 1];
  let lo = Math.min(...values);
  let hi = Math.max(...values);
  if (hi - lo < 1) {
    lo -= 0.5;
    hi += 0.5;
  }
  const span = hi - lo;
  lo -= span * 0.08;
  hi += span * 0.08;
  const tMax = run?.readings.at(-1)?.t ?? 1;
  const x = (t: number) => PAD.left + (t / tMax) * (W - PAD.left - PAD.right);
  const y = (v: number) => PAD.top + (1 - (v - lo) / (hi - lo)) * (H - PAD.top - PAD.bottom);
  const yTicks = niceTicks(lo, hi, 4);
  const xTicks = [0, 2, 4, 6, 8, 10].map((m) => m * 60_000).filter((t) => t <= tMax);

  const publishedSet = new Map(run?.published.map((p) => [p.index, p]) ?? []);
  const heartbeats = run?.published.filter((p) => p.heartbeat).length ?? 0;
  const held = run?.published.filter((p) => p.at > run.readings[p.index].t).length ?? 0;
  const readings = run?.readings.length ?? 0;
  const published = run?.published.length ?? 0;
  const saved = readings ? Math.round((1 - published / readings) * 100) : 0;
  const toml = tableToToml(table as ReportTable);

  const onPointer = (e: PointerEvent) => {
    if (!run || !svgRef.current) return;
    const rect = svgRef.current.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * W;
    const t = ((px - PAD.left) / (W - PAD.left - PAD.right)) * tMax;
    const i = Math.round((t / tMax) * (run.readings.length - 1));
    setHover(Math.max(0, Math.min(run.readings.length - 1, i)));
  };
  const onKey = (e: KeyboardEvent) => {
    if (!run) return;
    const step = e.shiftKey ? 10 : 1;
    if (e.key === 'ArrowRight') setHover(Math.min(run.readings.length - 1, (hover ?? -1) + step));
    else if (e.key === 'ArrowLeft') setHover(Math.max(0, (hover ?? 1) - step));
    else return;
    e.preventDefault();
  };

  const linePath = run?.readings.map((r, i) => `${i ? 'L' : 'M'}${x(r.t).toFixed(1)},${y(r.value).toFixed(1)}`).join('') ?? '';
  const hovered = hover !== null && run ? run.readings[hover] : null;
  const hoveredPub = hover !== null ? publishedSet.get(hover) : undefined;

  return (
    <div class="td-tool rp">
      <div class="rp-presets" role="group" aria-label="Presets">
        {PRESETS.map((p) => (
          <button
            type="button"
            class="td-btn"
            aria-pressed={p.signal === signal && JSON.stringify(p.form) === JSON.stringify(form)}
            onClick={() => {
              setSignal(p.signal);
              setForm(p.form);
            }}
          >
            {p.label}
          </button>
        ))}
      </div>

      <div class="rp-grid">
        <form class="td-panel rp-form" onSubmit={(e) => e.preventDefault()}>
          <label class="td-field">
            <span>Signal</span>
            <select value={signal} onChange={(e) => setSignal((e.currentTarget as HTMLSelectElement).value as SignalId)}>
              {SIGNALS.map((s) => (
                <option value={s.id}>{s.label}</option>
              ))}
            </select>
            <small>Polled every second for 10 minutes.</small>
          </label>
          <label class="rp-check">
            <input type="checkbox" checked={form.on_change} onChange={set('on_change')} />
            <code>on_change</code>
          </label>
          <Field label="deadband" hint='A number, or a percentage like "2%"' value={form.deadband} onInput={set('deadband')} />
          <Field label="min_interval" hint='At most once per interval, e.g. "30s"' value={form.min_interval} onInput={set('min_interval')} />
          <Field label="max_interval" hint='Heartbeat, e.g. "30m"' value={form.max_interval} onInput={set('max_interval')} />
          <Field label="debounce" hint='Publish once settled, e.g. "5s"' value={form.debounce} onInput={set('debounce')} />
          {error ? (
            <p class="td-error" role="alert">{error}</p>
          ) : (
            <pre class="rp-toml" aria-label="The report table in TOML"><code>{toml}</code></pre>
          )}
        </form>

        <div class="rp-out">
          <dl class="rp-stats">
            <div><dt>Readings</dt><dd>{readings}</dd></div>
            <div><dt>Published</dt><dd>{published}</dd></div>
            <div><dt>Saved</dt><dd>{saved}%</dd></div>
            <div><dt>Heartbeats</dt><dd>{heartbeats}</dd></div>
          </dl>

          <ul class="rp-legend" aria-label="Legend">
            <li><svg width="22" height="10" aria-hidden="true"><line x1="0" y1="5" x2="22" y2="5" class="rp-line" /></svg>Reading</li>
            <li><svg width="12" height="12" aria-hidden="true"><circle cx="6" cy="6" r="4.5" class="rp-pub" /></svg>Published</li>
            <li><svg width="12" height="12" aria-hidden="true"><rect x="2" y="2" width="8" height="8" class="rp-hb" transform="rotate(45 6 6)" /></svg>Heartbeat</li>
            <li><svg width="22" height="10" aria-hidden="true"><line x1="0" y1="5" x2="22" y2="5" class="rp-delay" /></svg>Held, published later</li>
          </ul>

          <div class="rp-chart">
            <svg
              ref={svgRef}
              viewBox={`0 0 ${W} ${H}`}
              role="img"
              aria-label={`Chart: ${readings} readings, ${published} published, ${heartbeats} heartbeats. Use the arrow keys to inspect readings.`}
              tabIndex={0}
              onPointerMove={onPointer as any}
              onPointerLeave={() => setHover(null)}
              onKeyDown={onKey as any}
              onBlur={() => setHover(null)}
            >
              {yTicks.map((v) => (
                <g>
                  <line x1={PAD.left} x2={W - PAD.right} y1={y(v)} y2={y(v)} class="rp-grid-line" />
                  <text x={PAD.left - 6} y={y(v)} class="rp-axis" text-anchor="end" dominant-baseline="middle">{fmt(v)}</text>
                </g>
              ))}
              {xTicks.map((t) => (
                <text x={x(t)} y={H - 8} class="rp-axis" text-anchor="middle">{t / 60_000} min</text>
              ))}
              <path d={linePath} class="rp-line" />
              {run?.published.map((p) => {
                const r = run.readings[p.index];
                return p.at > r.t ? <line x1={x(r.t)} x2={x(p.at)} y1={y(r.value)} y2={y(r.value)} class="rp-delay" /> : null;
              })}
              {run?.published.map((p) => {
                const r = run.readings[p.index];
                return p.heartbeat ? (
                  <rect x={x(r.t) - 4} y={y(r.value) - 4} width="8" height="8" class="rp-hb" transform={`rotate(45 ${x(r.t)} ${y(r.value)})`} />
                ) : (
                  <circle cx={x(r.t)} cy={y(r.value)} r="4.5" class="rp-pub" />
                );
              })}
              {hovered && (
                <line x1={x(hovered.t)} x2={x(hovered.t)} y1={PAD.top} y2={H - PAD.bottom} class="rp-crosshair" />
              )}
            </svg>
            {hovered && (
              <div class="rp-tip" style={{ left: `${(x(hovered.t) / W) * 100}%` }} aria-live="polite">
                <strong>{fmt(hovered.value)}{unit && ` ${unit}`}</strong>
                <span>at {fmtTime(hovered.t)}</span>
                <span>
                  {hoveredPub
                    ? hoveredPub.heartbeat
                      ? 'Published: heartbeat'
                      : hoveredPub.at > hovered.t
                        ? `Published at ${fmtTime(hoveredPub.at)} (held back)`
                        : 'Published'
                    : 'Not published'}
                </span>
              </div>
            )}
          </div>
          {held > 0 && <p class="rp-note">{held} reading{held === 1 ? ' was' : 's were'} held back and published once the interval ended, with {held === 1 ? 'its' : 'their'} own timestamp.</p>}

          <details class="rp-table">
            <summary>Published samples as a table</summary>
            <div class="rp-table-scroll">
              <table>
                <thead><tr><th>Reading time</th><th>Value</th><th>Published at</th><th>Why</th></tr></thead>
                <tbody>
                  {run?.published.map((p) => {
                    const r = run.readings[p.index];
                    return (
                      <tr>
                        <td>{fmtTime(r.t)}</td>
                        <td>{fmt(r.value)}</td>
                        <td>{fmtTime(p.at)}</td>
                        <td>{p.heartbeat ? 'heartbeat' : p.at > r.t ? 'held, then published' : 'change'}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </details>
        </div>
      </div>
    </div>
  );
}

function Field(props: { label: string; hint: string; value: string; onInput: (e: Event) => void }) {
  return (
    <label class="td-field">
      <span><code>{props.label}</code></span>
      <input type="text" value={props.value} onInput={props.onInput} placeholder="off" spellcheck={false} />
      <small>{props.hint}</small>
    </label>
  );
}

function niceTicks(lo: number, hi: number, count: number): number[] {
  const raw = (hi - lo) / count;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw) ?? raw;
  const out: number[] = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi; v += step) out.push(Math.round(v * 1e6) / 1e6);
  return out;
}

function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : v.toFixed(2);
}

function fmtTime(ms: number): string {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}
