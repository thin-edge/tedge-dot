/**
 * The form view's logic, kept out of the component so it can be tested on its own.
 *
 * The form edits the parsed connector file with the docs site's config builder model
 * (src/generated/config-builder.ts) and writes it back with the builder's `emitToml`. That layout
 * has no comments, so a file whose text is not already in it loses its comments and formatting
 * on the first form edit: `rewriteChanges` tells the editor when to ask first.
 */
import { emitToml, parseToml, protocolInfo, setAt, type Config } from '../generated/config-builder';

/** How the form edits a value. */
export type FieldKind = 'text' | 'integer' | 'number' | 'choice' | 'check';

export interface ShapeField {
  key: string;
  kind: FieldKind;
  options?: string[];
  placeholder?: string;
}

/**
 * The fields of a protocol table (`protocol_address`, a point's `address`, `[connection]`) from
 * the builder's protocol shape, in the shape's order.
 */
export function shapeFields(schema: any): ShapeField[] {
  return Object.entries<any>(schema?.properties ?? {}).map(([key, s]) => {
    if (Array.isArray(s?.enum)) {
      return { key, kind: 'choice', options: s.enum.map(String) };
    }
    if (s?.type === 'boolean') {
      return { key, kind: 'check' };
    }
    const kind: FieldKind = s?.type === 'integer' || s?.type === 'number' ? s.type : 'text';
    const placeholder = typeof s?.description === 'string' ? s.description.slice(0, 24) : undefined;
    return { key, kind, placeholder };
  });
}

/** The keys of a table that the form has no field for, in the table's order. */
export function extraKeys(table: unknown, known: readonly string[]): string[] {
  if (!table || typeof table !== 'object' || Array.isArray(table)) {
    return [];
  }
  return Object.keys(table).filter(k => !known.includes(k));
}

/** Whether a form edit would change more than the edited value: comments or layout would go. */
export function rewriteChanges(text: string): boolean {
  return emitToml(parseToml(text)) !== text;
}

/** The lines of a TOML text that hold a comment, whole-line or after a value. */
export function commentLines(text: string): number {
  let count = 0;
  let multiline: '"""' | "'''" | undefined;
  for (const line of text.split('\n')) {
    let i = 0;
    let commented = false;
    while (i < line.length) {
      if (multiline) {
        const end = line.indexOf(multiline, i);
        if (end < 0) {
          break;
        }
        i = end + 3;
        multiline = undefined;
        continue;
      }
      const rest = line.slice(i);
      if (rest.startsWith('"""') || rest.startsWith("'''")) {
        multiline = rest.slice(0, 3) as '"""' | "'''";
        i += 3;
        continue;
      }
      const c = line[i];
      if (c === '#') {
        commented = true;
        break;
      }
      if (c === '"' || c === "'") {
        // A one-line string: skip to its end; a basic string may escape its quote.
        i++;
        while (i < line.length && line[i] !== c) {
          i += c === '"' && line[i] === '\\' ? 2 : 1;
        }
      }
      i++;
    }
    if (commented) {
      count++;
    }
  }
  return count;
}

/** `base`, or `base_2`, `base_3`, ... when `base` is taken. */
export function uniqueId(base: string, taken: readonly unknown[]): string {
  if (!taken.includes(base)) {
    return base;
  }
  let n = 2;
  while (taken.includes(`${base}_${n}`)) {
    n++;
  }
  return `${base}_${n}`;
}

/** A comma-separated list as its items, without blanks. */
export function splitList(text: string): string[] {
  return text
    .split(',')
    .map(s => s.trim())
    .filter(Boolean);
}

const devicesOf = (config: Config): Config[] => (Array.isArray(config.device) ? config.device : []);
const pointsOf = (device: Config | undefined): Config[] => (Array.isArray(device?.point) ? device.point : []);

/** A new device from the protocol's template, named after the template with the next number. */
export function addDevice(config: Config): Config {
  const template = structuredClone(protocolInfo(config.connector?.protocol ?? '').newDevice);
  const devices = devicesOf(config);
  const stem = String(template.name ?? 'device').replace(/\d+$/, '');
  const name = uniqueId(`${stem}${devices.length + 1}`, devices.map(d => d.name));
  return setAt(config, `/device/${devices.length}`, { ...template, name });
}

/** A new point on device `d`, from the protocol's template, with an id no other point there has. */
export function addPoint(config: Config, d: number): Config {
  const template = structuredClone(protocolInfo(config.connector?.protocol ?? '').newPoint);
  const points = pointsOf(devicesOf(config)[d]);
  const id = uniqueId(String(template.id ?? 'point'), points.map(p => p.id));
  return setAt(config, `/device/${d}/point/${points.length}`, { ...template, id });
}

export function removeDevice(config: Config, d: number): Config {
  return { ...config, device: devicesOf(config).filter((_, i) => i !== d) };
}

export function removePoint(config: Config, d: number, p: number): Config {
  const devices = devicesOf(config);
  const device = devices[d];
  return setAt(
    config,
    `/device/${d}/point`,
    pointsOf(device).filter((_, i) => i !== p)
  );
}

/**
 * The configuration on another protocol: every device and point gets the new protocol's address
 * template and the template's data type (an address and its data type belong together, as the
 * register count of a Modbus value shows), the protocol's own tables and keys go, and ids and
 * policies stay. A service name that was the old protocol's default becomes the new one's.
 */
export function switchProtocol(config: Config, protocol: string): Config {
  const info = protocolInfo(protocol);
  const old = config.connector?.protocol ?? '';
  const { sampling_interval, ...connector } = config.connector ?? {};
  const serviceName =
    connector.service_name === undefined || connector.service_name === `tedge-dot-${old}`
      ? `tedge-dot-${protocol}`
      : connector.service_name;
  const next: Config = {
    ...config,
    connector: {
      ...connector,
      protocol,
      service_name: serviceName,
      ...(protocol === 'opcua' && sampling_interval !== undefined ? { sampling_interval } : {}),
    },
    device: devicesOf(config).map(device => ({
      ...device,
      protocol_address: structuredClone(info.newDevice.protocol_address),
      point: pointsOf(device).map(point => {
        const { address, subscribe, sampling_interval: pointSampling, datatype, ...rest } = point;
        return {
          ...rest,
          ...(info.newPoint.datatype !== undefined ? { datatype: info.newPoint.datatype } : datatype !== undefined ? { datatype } : {}),
          ...(protocol === 'opcua' && pointSampling !== undefined ? { sampling_interval: pointSampling } : {}),
          ...(protocol === 'opcua' && subscribe !== undefined ? { subscribe } : {}),
          address: structuredClone(info.newPoint.address),
        };
      }),
    })),
  };
  if (!Array.isArray(config.device)) {
    delete next['device'];
  }
  delete next['connection'];
  return next;
}

/** Whether a point is pushed by the server rather than polled. */
export function isPushed(protocol: string, point: Config): boolean {
  return protocol === 'opcua' && point?.subscribe !== false;
}

/** A collapsed point at a glance: datatype, unit, how it is read, and the policies it sets itself. */
export function pointSummary(config: Config, device: Config, point: Config): string {
  const parts: string[] = [point.datatype, point.unit].filter(Boolean).map(String);
  const protocol = config.connector?.protocol ?? '';
  const interval = point.poll_interval ?? device?.poll_interval;
  if (isPushed(protocol, point)) {
    parts.push(`subscribed${point.sampling_interval ? ` ${point.sampling_interval}` : ''}`);
  } else {
    parts.push(interval ? `polled ${interval}` : 'polled');
  }
  if (point.report) {
    parts.push('own report');
  }
  if (point.grouping || point.batch) {
    parts.push('own grouping');
  }
  return parts.join(' · ');
}

/** Labels that sentence case gets wrong: acronyms, units, and terms with their own spelling. */
const LABELS: Record<string, string> = {
  id: 'ID',
  node_id: 'Node ID',
  unit_id: 'Unit ID',
  web_id: 'Web ID',
  oid: 'OID',
  tls: 'TLS',
  dbc_file: 'DBC file',
  af_database: 'AF database',
  application_uri: 'Application URI',
  base_url: 'Base URL',
  connect_timeout_s: 'Connect timeout (s)',
  request_timeout_s: 'Request timeout (s)',
  baudrate: 'Baud rate',
  databits: 'Data bits',
  stopbits: 'Stop bits',
  datatype: 'Data type',
  mqtt: 'MQTT',
  max_inflight: 'Max. requests in flight',
  max_inflight_total: 'Max. requests in flight, total',
};

/** A TOML key as a form label, in sentence case: `poll_interval` is "Poll interval". */
export function labelOf(key: string): string {
  const known = LABELS[key];
  if (known) {
    return known;
  }
  const words = key.replace(/[_.]+/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

/** A value as a text field shows it: nothing for no value, JSON for tables and arrays. */
export function displayValue(value: unknown): string {
  if (value === undefined || value === null) {
    return '';
  }
  return typeof value === 'object' ? JSON.stringify(value) : String(value);
}
