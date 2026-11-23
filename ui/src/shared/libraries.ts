/**
 * Point libraries (contract §3.4) as the plugin knows them: the repository's library entries and
 * the libraries the tedge-dot packages install. A connector file may only reference those, and the
 * form shows the points a device inherits from them.
 *
 * Resolution follows tedge-dot's own (impl/rust/crates/sdk/src/library.rs): libraries in the order
 * a device lists them, then the device's own points; a point whose id exists patches it, with
 * `meta`, `transform`, `report` and `grouping` merged key by key (recursively) and every other
 * field replaced. It only drives what the form shows and the reference check: the gateway
 * resolves on its own.
 */
import { parseToml, type Config } from '../generated/config-builder';
import packaged from '../generated/packaged-libraries.json';
import { type ManagedFile, libraryFile } from './naming';
import { type FieldProblem, describePointer } from './validation';

/** The point fields that patch an inherited point key by key instead of replacing it. */
export const DEEP_MERGED_KEYS = ['meta', 'transform', 'report', 'grouping'];

export type LibrarySource = 'repository' | 'packaged';

export interface KnownLibrary {
  protocol: string;
  name: string;
  source: LibrarySource;
  /** The parsed library, when its text could be read. */
  config?: Config;
}

/** The libraries a connector file may reference, by protocol and name. */
export class LibraryCatalog {
  private readonly byKey = new Map<string, KnownLibrary>();

  /** A site library shadows a packaged one of the same name, as on the gateway. */
  constructor(libraries: readonly KnownLibrary[]) {
    for (const library of libraries) {
      const key = `${library.protocol}/${library.name}`;
      const known = this.byKey.get(key);
      if (!known || (known.source === 'packaged' && library.source === 'repository')) {
        this.byKey.set(key, library);
      }
    }
  }

  get(protocol: string, name: string): KnownLibrary | undefined {
    return this.byKey.get(`${protocol}/${name}`);
  }

  /** The library names available for `protocol`, sorted. */
  names(protocol: string): string[] {
    return [...this.byKey.values()]
      .filter(l => l.protocol === protocol)
      .map(l => l.name)
      .sort();
  }

  all(): KnownLibrary[] {
    return [...this.byKey.values()];
  }
}

/** The libraries the tedge-dot packages install, as the build step copied them. */
export function packagedLibraries(): KnownLibrary[] {
  return (packaged as { protocol: string; name: string; text: string }[]).map(({ protocol, name, text }) => {
    let config: Config | undefined;
    try {
      config = parseToml(text);
    } catch {
      config = undefined;
    }
    return { protocol, name, source: 'packaged', config };
  });
}

/** True when a `points_from` entry is a path rather than a library name (as tedge-dot decides). */
export function isPathReference(reference: string): boolean {
  return reference.endsWith('.toml') || reference.includes('/');
}

/** The library names a connector file's devices reference, in order, without repeats. */
export function referencedLibraries(config: Config): string[] {
  const names: string[] = [];
  for (const device of Array.isArray(config['device']) ? (config['device'] as Config[]) : []) {
    for (const ref of Array.isArray(device['points_from']) ? device['points_from'] : []) {
      if (typeof ref === 'string' && !isPathReference(ref) && !names.includes(ref)) {
        names.push(ref);
      }
    }
  }
  return names;
}

/**
 * The repository libraries a connector file references, as files to register and send: the names
 * whose library type is among `repositoryTypes`. Packaged libraries are on every gateway, and
 * unknown names are left to the reference check.
 */
export function referencedLibraryFiles(config: Config, repositoryTypes: Iterable<string>): ManagedFile[] {
  const protocol = config['connector']?.protocol;
  if (typeof protocol !== 'string') {
    return [];
  }
  const available = new Set(repositoryTypes);
  return referencedLibraries(config)
    .map(name => libraryFile(protocol, name))
    .filter(file => available.has(file.type));
}

/**
 * The references the plugin cannot accept: a path, or a name no known library of the file's
 * protocol has. Each problem is reported at the device's `points_from`.
 */
export function checkReferences(config: Config, catalog: LibraryCatalog): FieldProblem[] {
  const protocol = config['connector']?.protocol ?? '';
  const problems: FieldProblem[] = [];
  (Array.isArray(config['device']) ? (config['device'] as Config[]) : []).forEach((device, d) => {
    const refs = Array.isArray(device['points_from']) ? device['points_from'] : [];
    const path = `/device/${d}/points_from`;
    for (const ref of refs) {
      if (typeof ref !== 'string') {
        continue;
      }
      let message: string | undefined;
      if (isPathReference(ref)) {
        message = `"${ref}" is a path; reference a point library by its name`;
      } else if (!catalog.get(protocol, ref)) {
        message = `no ${protocol} point library is named "${ref}"`;
      }
      if (message) {
        problems.push({ path, message, label: describePointer(config, path) });
      }
    }
  });
  return problems;
}

export interface InheritedPoint {
  /** The point as the device gets it, after every library that defines or patches it. */
  point: Config;
  /** The library that defines it first. */
  library: string;
}

export interface ResolvedDevice {
  /** The inherited points the device does not override, in order. */
  inherited: InheritedPoint[];
  /** For each of the device's own points that overrides an inherited one: that point's library. */
  overrides: Map<number, string>;
  /** The device type the device inherits, when it declares none. */
  type?: { name: string; library: string };
  /** References no known library resolves. */
  missing: string[];
}

/** What a device inherits from the libraries it references (contract §3.4). */
export function resolveDevice(device: Config, protocol: string, catalog: LibraryCatalog): ResolvedDevice {
  const points: InheritedPoint[] = [];
  const index = new Map<string, number>();
  const missing: string[] = [];
  let type: ResolvedDevice['type'];

  for (const ref of Array.isArray(device['points_from']) ? device['points_from'] : []) {
    if (typeof ref !== 'string') {
      continue;
    }
    const library = catalog.get(protocol, ref)?.config;
    if (!library) {
      missing.push(ref);
      continue;
    }
    const declared = library['library']?.type;
    if (!type && typeof declared === 'string' && declared.trim()) {
      type = { name: declared.trim(), library: ref };
    }
    for (const point of Array.isArray(library['point']) ? (library['point'] as Config[]) : []) {
      const id = point?.['id'];
      const at = typeof id === 'string' ? index.get(id) : undefined;
      if (at === undefined) {
        if (typeof id === 'string') {
          index.set(id, points.length);
        }
        points.push({ point: structuredClone(point), library: ref });
      } else {
        points[at] = { ...points[at], point: mergePoint(points[at].point, point) };
      }
    }
  }

  const overrides = new Map<number, string>();
  const overridden = new Set<number>();
  (Array.isArray(device['point']) ? (device['point'] as Config[]) : []).forEach((point, p) => {
    const id = point?.['id'];
    const at = typeof id === 'string' ? index.get(id) : undefined;
    if (at !== undefined) {
      overrides.set(p, points[at].library);
      overridden.add(at);
    }
  });

  const deviceType = device['type'];
  return {
    inherited: points.filter((_, i) => !overridden.has(i)),
    overrides,
    type: typeof deviceType === 'string' && deviceType.trim() ? undefined : type,
    missing,
  };
}

/** A point patched by another definition of the same id (contract §3.4, `merge_point_table`). */
export function mergePoint(base: Config, over: Config): Config {
  const merged: Config = structuredClone(base);
  for (const [key, value] of Object.entries(over)) {
    if (DEEP_MERGED_KEYS.includes(key) && isTable(merged[key]) && isTable(value)) {
      merged[key] = mergeTable(merged[key], value);
    } else {
      merged[key] = structuredClone(value);
    }
  }
  return merged;
}

function mergeTable(base: Config, over: Config): Config {
  const merged: Config = { ...base };
  for (const [key, value] of Object.entries(over)) {
    merged[key] = isTable(merged[key]) && isTable(value) ? mergeTable(merged[key], value) : structuredClone(value);
  }
  return merged;
}

function isTable(value: unknown): value is Config {
  return typeof value === 'object' && value !== null && !Array.isArray(value) && !(value instanceof Date);
}
