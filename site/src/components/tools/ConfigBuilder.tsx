/** @jsxImportSource preact */
// The config builder: a form over a connector configuration, the TOML it makes, and the
// contract schema's verdict on it, live. The model and validation are src/lib/config-builder.ts.

import { useMemo, useState } from 'preact/hooks';
import {
  coerce,
  emitToml,
  getAt,
  newConfig,
  parseToml,
  protocolInfo,
  PROTOCOL_IDS,
  SCHEMA,
  setAt,
  validateConfig,
  type Config,
  type FieldError,
  type ProtocolId,
} from '../../lib/config-builder';
import './config-builder.css';

const POINT = SCHEMA.$defs.point.properties;
const DEVICE = SCHEMA.$defs.device.properties;
const CONNECTOR = SCHEMA.properties.connector.properties;

interface Ctx {
  config: Config;
  errors: Map<string, string>;
  set: (path: string, value: unknown) => void;
}

export default function ConfigBuilder({ demos }: { demos: Record<string, string> }) {
  const [config, setConfig] = useState<Config>(() => newConfig('modbus'));
  const [notice, setNotice] = useState<string>('');
  const [importText, setImportText] = useState('');
  const [importError, setImportError] = useState('');

  const toml = useMemo(() => emitToml(config), [config]);
  const errorList = useMemo(() => validateConfig(config), [config]);
  const errors = new Map(errorList.map((e) => [e.path, e.message]));
  const ctx: Ctx = { config, errors, set: (path, value) => setConfig((c) => setAt(c, path, value)) };
  const protocol = config.connector?.protocol as ProtocolId;
  const info = protocolInfo(protocol);
  const devices: any[] = config.device ?? [];

  const load = (text: string, from: string) => {
    try {
      setConfig(parseToml(text));
      setNotice(`Loaded ${from}.`);
      return true;
    } catch (e) {
      setImportError(String((e as Error).message ?? e));
      return false;
    }
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(toml);
      setNotice(errorList.length ? 'Copied — but the config does not validate yet: fix the marked fields first.' : 'Copied to the clipboard.');
    } catch {
      setNotice('Copying is blocked here; select the text and copy it instead.');
    }
  };
  const download = () => {
    const url = URL.createObjectURL(new Blob([toml], { type: 'application/toml' }));
    const a = document.createElement('a');
    a.href = url;
    a.download = `${protocol || 'connector'}.toml`;
    a.click();
    URL.revokeObjectURL(url);
    if (errorList.length) setNotice('Downloaded — but the config does not validate yet.');
  };

  return (
    <div class="td-tool cb">
      <div class="cb-bar">
        <label class="td-field cb-inline">
          <span>Protocol</span>
          <select
            value={protocol}
            onChange={(e) => {
              setConfig(newConfig((e.currentTarget as HTMLSelectElement).value as ProtocolId));
              setNotice('');
            }}
          >
            {PROTOCOL_IDS.map((id) => <option value={id}>{protocolInfo(id).label}</option>)}
          </select>
        </label>
        <label class="td-field cb-inline">
          <span>Start from a demo</span>
          <select
            value=""
            onChange={(e) => {
              const name = (e.currentTarget as HTMLSelectElement).value;
              if (name) load(demos[name], `demo/config/${name}.toml`);
            }}
          >
            <option value="">Choose…</option>
            {Object.keys(demos).map((name) => <option value={name}>{name}</option>)}
          </select>
        </label>
        <details class="cb-import">
          <summary class="td-btn">Paste TOML</summary>
          <div class="td-panel">
            <textarea rows={8} value={importText} onInput={(e) => setImportText((e.currentTarget as HTMLTextAreaElement).value)} aria-label="TOML to load" spellcheck={false} />
            {importError && <p class="td-error">{importError}</p>}
            <button type="button" class="td-btn" onClick={() => { setImportError(''); load(importText, 'your TOML'); }}>Load</button>
          </div>
        </details>
      </div>

      <div class="cb-grid">
        <form class="cb-form" onSubmit={(e) => e.preventDefault()} aria-label="Connector configuration">
          <fieldset class="td-panel">
            <legend>[connector]</legend>
            <div class="cb-row">
              <Text ctx={ctx} path="/connector/service_name" label="service_name" hint={CONNECTOR.service_name.description} />
              <Text ctx={ctx} path="/connector/poll_interval" label="poll_interval" placeholder="2s" />
              <Choice ctx={ctx} path="/connector/log_level" label="log_level" options={CONNECTOR.log_level.enum} />
            </div>
            <Report ctx={ctx} base="/connector/report" />
          </fieldset>

          <fieldset class="td-panel">
            <legend>[mqtt]</legend>
            <div class="cb-row">
              <Text ctx={ctx} path="/mqtt/host" label="host" placeholder="127.0.0.1" />
              <Text ctx={ctx} path="/mqtt/port" label="port" type="integer" placeholder="1883" />
            </div>
          </fieldset>

          {devices.map((device, d) => (
            <fieldset class="td-panel cb-device">
              <legend>[[device]] {device.name || `#${d + 1}`}</legend>
              <div class="cb-row">
                <Text ctx={ctx} path={`/device/${d}/name`} label="name" />
                <Text ctx={ctx} path={`/device/${d}/type`} label="type" hint={DEVICE.type.description} />
                <Text ctx={ctx} path={`/device/${d}/poll_interval`} label="poll_interval" placeholder="inherit" />
                <Choice ctx={ctx} path={`/device/${d}/default_mode`} label="default_mode" options={DEVICE.default_mode.enum} />
              </div>
              <List ctx={ctx} path={`/device/${d}/points_from`} label="points_from" hint="Point libraries, comma-separated (e.g. demo-sim)" />
              <Shape ctx={ctx} base={`/device/${d}/protocol_address`} schema={info?.protocol_address} legend="protocol_address" />

              <h4 class="cb-points-title">Points</h4>
              {(device.point ?? []).map((_: any, p: number) => (
                <div class="cb-point">
                  <div class="cb-row">
                    <Text ctx={ctx} path={`/device/${d}/point/${p}/id`} label="id" />
                    <Choice ctx={ctx} path={`/device/${d}/point/${p}/datatype`} label="datatype" options={POINT.datatype.enum} />
                    <Choice ctx={ctx} path={`/device/${d}/point/${p}/access`} label="access" options={POINT.access.enum} />
                    <Text ctx={ctx} path={`/device/${d}/point/${p}/unit`} label="unit" />
                  </div>
                  <Shape ctx={ctx} base={`/device/${d}/point/${p}/address`} schema={info?.address} legend="address" />
                  <button type="button" class="td-btn cb-remove" onClick={() => ctx.set(`/device/${d}/point`, device.point.filter((_: any, i: number) => i !== p))}>
                    Remove point
                  </button>
                </div>
              ))}
              <div class="cb-actions">
                <button type="button" class="td-btn" onClick={() => ctx.set(`/device/${d}/point/${(device.point ?? []).length}`, structuredClone(info.newPoint))}>
                  + Add point
                </button>
                <button type="button" class="td-btn" onClick={() => ctx.set('/device', devices.filter((_, i) => i !== d))}>
                  Remove device
                </button>
              </div>
            </fieldset>
          ))}
          <button type="button" class="td-btn" onClick={() => ctx.set(`/device/${devices.length}`, { ...structuredClone(info.newDevice), name: `${info.newDevice.name.replace(/\d+$/, '')}${devices.length + 1}` })}>
            + Add device
          </button>
        </form>

        <div class="cb-out">
          <div class={`cb-status ${errorList.length ? 'bad' : 'ok'}`} role="status">
            {errorList.length ? `${errorList.length} problem${errorList.length > 1 ? 's' : ''}` : 'Valid against the contract schema'}
          </div>
          {errorList.length > 0 && <Problems errors={errorList} />}
          <pre class="cb-toml" aria-label="Generated TOML"><code>{toml}</code></pre>
          <div class="cb-actions">
            <button type="button" class="td-btn primary" onClick={copy}>{errorList.length ? 'Copy (invalid)' : 'Copy'}</button>
            <button type="button" class="td-btn" onClick={download}>Download {protocol}.toml</button>
          </div>
          {notice && <p class="cb-notice" aria-live="polite">{notice}</p>}
          <p class="cb-hint">
            The schema checks structure. For the full check (point libraries resolved, protocol
            rules), run <code>tedge-dot describe -c {protocol}.toml</code> on the gateway.
          </p>
        </div>
      </div>
    </div>
  );
}

function Problems({ errors }: { errors: FieldError[] }) {
  return (
    <ul class="cb-problems">
      {errors.slice(0, 8).map((e) => (
        <li><code>{e.path || '/'}</code> {e.message}</li>
      ))}
    </ul>
  );
}

function fieldId(path: string) {
  return `cb${path.replace(/[^a-z0-9]+/gi, '-')}`;
}

function Text(props: { ctx: Ctx; path: string; label: string; type?: string; hint?: string; placeholder?: string }) {
  const { ctx, path, label, type, hint, placeholder } = props;
  const value = getAt(ctx.config, path);
  const error = ctx.errors.get(path);
  const id = fieldId(path);
  return (
    <div class={`td-field ${error ? 'has-error' : ''}`}>
      <label for={id}><code>{label}</code></label>
      <input
        id={id}
        type="text"
        inputMode={type === 'integer' || type === 'number' ? 'numeric' : undefined}
        value={value === undefined ? '' : typeof value === 'object' ? JSON.stringify(value) : String(value)}
        placeholder={placeholder}
        aria-invalid={error ? 'true' : undefined}
        aria-describedby={error ? `${id}-err` : undefined}
        title={hint}
        spellcheck={false}
        onInput={(e) => ctx.set(path, coerce((e.currentTarget as HTMLInputElement).value, type))}
      />
      {error && <span id={`${id}-err`} class="td-error">{error}</span>}
    </div>
  );
}

function Choice(props: { ctx: Ctx; path: string; label: string; options: unknown[] }) {
  const { ctx, path, label, options } = props;
  const value = getAt(ctx.config, path);
  const error = ctx.errors.get(path);
  const id = fieldId(path);
  return (
    <div class={`td-field ${error ? 'has-error' : ''}`}>
      <label for={id}><code>{label}</code></label>
      <select id={id} value={value === undefined ? '' : String(value)} onChange={(e) => ctx.set(path, (e.currentTarget as HTMLSelectElement).value || undefined)}>
        <option value="">—</option>
        {options.map((o) => <option value={String(o)}>{String(o)}</option>)}
      </select>
      {error && <span class="td-error">{error}</span>}
    </div>
  );
}

function Check(props: { ctx: Ctx; path: string; label: string }) {
  const value = getAt(props.ctx.config, props.path);
  return (
    <label class="cb-check">
      <input type="checkbox" checked={value === true} onChange={(e) => props.ctx.set(props.path, (e.currentTarget as HTMLInputElement).checked || undefined)} />
      <code>{props.label}</code>
    </label>
  );
}

function List(props: { ctx: Ctx; path: string; label: string; hint: string }) {
  const { ctx, path, label, hint } = props;
  const value: string[] | undefined = getAt(ctx.config, path);
  const error = ctx.errors.get(path);
  const [text, setText] = useState((value ?? []).join(', '));
  const id = fieldId(path);
  return (
    <div class={`td-field ${error ? 'has-error' : ''}`}>
      <label for={id}><code>{label}</code></label>
      <input
        id={id}
        type="text"
        // Keep what is being typed ("a, ") unless the value changed elsewhere (a demo loaded).
        value={JSON.stringify(splitList(text)) === JSON.stringify(value ?? []) ? text : (value ?? []).join(', ')}
        spellcheck={false}
        onInput={(e) => {
          const t = (e.currentTarget as HTMLInputElement).value;
          setText(t);
          const items = splitList(t);
          ctx.set(path, items.length ? items : undefined);
        }}
      />
      <small>{hint}</small>
      {error && <span class="td-error">{error}</span>}
    </div>
  );
}

function splitList(text: string): string[] {
  return text.split(',').map((s) => s.trim()).filter(Boolean);
}

/** The fields of a protocol-specific table, from the builder's protocol shapes. */
function Shape(props: { ctx: Ctx; base: string; schema: any; legend: string }) {
  const { ctx, base, schema, legend } = props;
  const props_ = Object.entries<any>(schema?.properties ?? {});
  const value = getAt(ctx.config, base) ?? {};
  const known = new Set(props_.map(([k]) => k));
  const extra = Object.keys(value).filter((k) => !known.has(k));
  const error = ctx.errors.get(base);
  return (
    <div class="cb-shape">
      <p class="cb-shape-title"><code>{legend}</code>{error && <span class="td-error"> {error}</span>}</p>
      <div class="cb-row">
        {props_.map(([k, s]) =>
          s.enum ? (
            <Choice ctx={ctx} path={`${base}/${k}`} label={k} options={s.enum} />
          ) : s.type === 'boolean' ? (
            <Check ctx={ctx} path={`${base}/${k}`} label={k} />
          ) : (
            <Text ctx={ctx} path={`${base}/${k}`} label={k} type={s.type} hint={s.description} placeholder={s.description?.slice(0, 24)} />
          ),
        )}
      </div>
      {extra.length > 0 && (
        <p class="cb-extra">Also set, kept as is: {extra.map((k) => <code>{k}</code>)}</p>
      )}
    </div>
  );
}

function Report(props: { ctx: Ctx; base: string }) {
  const { ctx, base } = props;
  return (
    <details class="cb-report" open={getAt(ctx.config, base) !== undefined}>
      <summary><code>report</code> — reporting policy</summary>
      <div class="cb-row">
        <Check ctx={ctx} path={`${base}/on_change`} label="on_change" />
        <Text ctx={ctx} path={`${base}/deadband`} label="deadband" type="number" placeholder='0.5 or "2%"' />
        <Text ctx={ctx} path={`${base}/min_interval`} label="min_interval" placeholder="off" />
        <Text ctx={ctx} path={`${base}/max_interval`} label="max_interval" placeholder="30m" />
        <Text ctx={ctx} path={`${base}/debounce`} label="debounce" placeholder="off" />
      </div>
      <small>Try it first in the <a href="/tedge-dot/tools/report-playground/">report-policy playground</a>.</small>
    </details>
  );
}
