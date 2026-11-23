/**
 * How connector files and point libraries are named in Cumulocity.
 *
 * The file /etc/tedge/plugins/ot/<name>.toml is the configuration type `tedge-dot-<name>`. The
 * OPC-UA solution blueprint already registers opcua-pump.toml as `tedge-dot-opcua-pump`, so its
 * gateways work as they are. A point library /etc/tedge/plugins/ot/points.d/<protocol>/<name>.toml
 * is the type `tedge-dot-points.<protocol>.<name>`; connector file names may not start with
 * `points.`, so the two never share a type. A name is a plain file name, so it can never leave
 * its directory.
 *
 * Plain TypeScript, without Angular, so it is unit-tested as it is.
 */

/** Prefix of every connector file's configuration type. */
export const TYPE_PREFIX = 'tedge-dot-';

/** Where tedge-dot reads its connector files. */
export const CONNECTOR_DIR = '/etc/tedge/plugins/ot';

/** Prefix of every point library's configuration type. */
export const LIBRARY_PREFIX = 'tedge-dot-points.';

/** Where tedge-dot looks for a site's point libraries, one folder per protocol. */
export const POINTS_DIR = `${CONNECTOR_DIR}/points.d`;

/** The configuration type under which thin-edge.io offers its own list of managed files. */
export const REGISTRATION_TYPE = 'tedge-configuration-plugin';

const NAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/;
const MAX_NAME = 100;

/** A name as a user may type it: surrounding spaces and a trailing `.toml` are dropped. */
export function normalizeName(input: string): string {
  const trimmed = input.trim();
  return trimmed.toLowerCase().endsWith('.toml') ? trimmed.slice(0, -'.toml'.length) : trimmed;
}

/** Whether `name` is a file name the connector directory can hold under this scheme. */
export function isValidName(name: string): boolean {
  return name.length <= MAX_NAME && NAME.test(name);
}

/** The configuration type of the connector file `name`. */
export function typeOf(name: string): string {
  return TYPE_PREFIX + name;
}

/** Whether `name` can name a connector file: a valid name that does not start with `points.`. */
export function isValidConnectorName(name: string): boolean {
  return isValidName(name) && !name.startsWith('points.');
}

/** Whether `protocol` can be part of a library type: the form of the protocol ids. */
export function isValidProtocol(protocol: string): boolean {
  return /^[a-z0-9][a-z0-9-]*$/.test(protocol);
}

/** The connector file name of a configuration type, or undefined for any other type. */
export function nameOf(type: string | undefined | null): string | undefined {
  if (!type || !type.startsWith(TYPE_PREFIX) || type.startsWith(LIBRARY_PREFIX)) {
    return undefined;
  }
  const name = type.slice(TYPE_PREFIX.length);
  return isValidConnectorName(name) ? name : undefined;
}

/** Whether a configuration type is a connector file's. */
export function isConnectorType(type: string | undefined | null): boolean {
  return nameOf(type) !== undefined;
}

/** The path of the connector file `name` on the gateway. */
export function pathOf(name: string): string {
  return `${CONNECTOR_DIR}/${name}.toml`;
}

/** The configuration type of the point library `name` of `protocol`. */
export function libraryTypeOf(protocol: string, name: string): string {
  return `${LIBRARY_PREFIX}${protocol}.${name}`;
}

/** The protocol and name of a library type, or undefined for any other type. */
export function libraryOf(type: string | undefined | null): { protocol: string; name: string } | undefined {
  if (!type || !type.startsWith(LIBRARY_PREFIX)) {
    return undefined;
  }
  const rest = type.slice(LIBRARY_PREFIX.length);
  const dot = rest.indexOf('.');
  if (dot < 0) {
    return undefined;
  }
  const protocol = rest.slice(0, dot);
  const name = rest.slice(dot + 1);
  return isValidProtocol(protocol) && isValidName(name) ? { protocol, name } : undefined;
}

/** Whether a configuration type is a point library's. */
export function isLibraryType(type: string | undefined | null): boolean {
  return libraryOf(type) !== undefined;
}

/** The path of the point library `name` of `protocol` on the gateway. */
export function libraryPathOf(protocol: string, name: string): string {
  return `${POINTS_DIR}/${protocol}/${name}.toml`;
}

export type FileKind = 'connector' | 'library';

/** A file this plugin manages on gateways: a connector file or a point library. */
export interface ManagedFile {
  kind: FileKind;
  /** The file name without `.toml`. */
  name: string;
  /** For a library: the protocol its folder is named after. */
  protocol?: string;
  type: string;
  path: string;
}

export function connectorFile(name: string): ManagedFile {
  return { kind: 'connector', name, type: typeOf(name), path: pathOf(name) };
}

export function libraryFile(protocol: string, name: string): ManagedFile {
  return { kind: 'library', name, protocol, type: libraryTypeOf(protocol, name), path: libraryPathOf(protocol, name) };
}

/** The file a configuration type stands for, or undefined for a type this plugin does not manage. */
export function fileOfType(type: string | undefined | null): ManagedFile | undefined {
  const library = libraryOf(type);
  if (library) {
    return libraryFile(library.protocol, library.name);
  }
  const name = nameOf(type);
  return name === undefined ? undefined : connectorFile(name);
}

/** The part of a repository entry the helpers below need. */
export interface EntryLike {
  configurationType?: string;
  lastUpdated?: string;
}

/** The entries of one kind, in their order. */
export function entriesOfKind<T extends EntryLike>(entries: readonly T[], kind: FileKind): T[] {
  return entries.filter(e => fileOfType(e.configurationType)?.kind === kind);
}

/**
 * The newest entry of each type. The repository may hold several entries of one type; sending a
 * library or offering it in a list takes the most recently updated one.
 */
export function newestByType<T extends EntryLike>(entries: readonly T[]): Map<string, T> {
  const newest = new Map<string, T>();
  for (const entry of entries) {
    const type = entry.configurationType;
    if (!type) {
      continue;
    }
    const known = newest.get(type);
    if (!known || (entry.lastUpdated ?? '') > (known.lastUpdated ?? '')) {
      newest.set(type, entry);
    }
  }
  return newest;
}
