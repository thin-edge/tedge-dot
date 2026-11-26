/**
 * A new connector file's service name follows its file name, `tedge-dot-<file name>`, until the
 * user sets another one. The templates name the service after the protocol, as the packaged
 * example files do (`tedge-dot-opcua` in opcua.toml). A new file that kept that name would share
 * the example's MQTT identity on a gateway that runs both, and the two would keep knocking each
 * other off the broker (thin-edge/tedge-dot#142).
 *
 * The editor also warns when a connector file uses the service name of an example file the
 * packages install, whatever the file name.
 *
 * Plain TypeScript, without Angular, so it is unit-tested as it is.
 */
import { PROTOCOL_IDS, parseToml, setAt } from '../generated/config-builder';
import packagedConnectors from '../generated/packaged-connectors.json';
import { isValidConnectorName, normalizeName } from './naming';
import { writeKeepingLayout } from './toml-write';

/** An example connector file the packages install in /etc/tedge/plugins/ot. */
export interface PackagedConnector {
  /** The file name without `.toml`. */
  file: string;
  serviceName: string;
}

const PACKAGED: readonly PackagedConnector[] = packagedConnectors;

/** The service name a connector file `name` gets by default. */
export function serviceNameOf(name: string): string {
  return `tedge-dot-${name}`;
}

/**
 * Whether `name` was set for the user, not by them: missing, a template's `tedge-dot-<protocol>`,
 * or `followed`, the name this rule set last.
 */
export function isAutomaticServiceName(name: unknown, followed?: string): boolean {
  if (name === undefined) {
    return true;
  }
  return typeof name === 'string' && (name === followed || PROTOCOL_IDS.some(p => name === serviceNameOf(p)));
}

/**
 * The text with the service name of `fileName`, when the file name is valid and the current
 * service name is automatic; undefined when nothing changes. Comments and layout stay.
 */
export function followFileName(text: string, fileName: string, followed?: string): { text: string; serviceName: string } | undefined {
  const name = normalizeName(fileName);
  if (!isValidConnectorName(name)) {
    return undefined;
  }
  const serviceName = serviceNameOf(name);
  try {
    const current = parseToml(text)?.connector?.service_name;
    if (current === serviceName || !isAutomaticServiceName(current, followed)) {
      return undefined;
    }
  } catch {
    return undefined;
  }
  const next = setServiceName(text, serviceName);
  return next === undefined ? undefined : { text: next, serviceName };
}

/**
 * The text with `[connector] service_name` set to `serviceName`, keeping comments and layout;
 * undefined for a file that does not parse, has no `[connector]` table, or cannot be written so.
 */
export function setServiceName(text: string, serviceName: string): string | undefined {
  try {
    const config = parseToml(text);
    if (!config?.connector) {
      return undefined;
    }
    return writeKeepingLayout(text, setAt(config, '/connector/service_name', serviceName));
  } catch {
    return undefined;
  }
}

/**
 * The packaged example file whose service name a connector file `fileName` uses, if any. A file
 * named like the example replaces it on the gateway, so it does not clash.
 */
export function packagedClash(
  serviceName: unknown,
  fileName: string,
  packaged: readonly PackagedConnector[] = PACKAGED
): PackagedConnector | undefined {
  const name = normalizeName(fileName);
  return packaged.find(p => p.serviceName === serviceName && p.file !== name);
}
