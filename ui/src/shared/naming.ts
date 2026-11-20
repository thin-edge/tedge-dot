/**
 * How connector files are named in Cumulocity.
 *
 * The file /etc/tedge/plugins/ot/<name>.toml is the configuration type `tedge-dot-<name>`. The
 * OPC-UA solution blueprint already registers opcua-pump.toml as `tedge-dot-opcua-pump`, so its
 * gateways work as they are. A name is a plain file name, so it can never leave the connector
 * directory.
 *
 * Plain TypeScript, without Angular, so it is unit-tested as it is.
 */

/** Prefix of every connector file's configuration type. */
export const TYPE_PREFIX = 'tedge-dot-';

/** Where tedge-dot reads its connector files. */
export const CONNECTOR_DIR = '/etc/tedge/plugins/ot';

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

/** The connector file name of a configuration type, or undefined for any other type. */
export function nameOf(type: string | undefined | null): string | undefined {
  if (!type || !type.startsWith(TYPE_PREFIX)) {
    return undefined;
  }
  const name = type.slice(TYPE_PREFIX.length);
  return isValidName(name) ? name : undefined;
}

/** Whether a configuration type is a connector file's. */
export function isConnectorType(type: string | undefined | null): boolean {
  return nameOf(type) !== undefined;
}

/** The path of the connector file `name` on the gateway. */
export function pathOf(name: string): string {
  return `${CONNECTOR_DIR}/${name}.toml`;
}
