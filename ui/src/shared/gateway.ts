/** Which devices get the OT connectors tab. Plain TypeScript so it is tested without Angular. */
import { REGISTRATION_TYPE, isConnectorType, isLibraryType } from './naming';

/** A thin-edge.io gateway: it offers thin-edge.io's own list of managed files. */
export function isThinEdgeGateway(device: unknown): boolean {
  const types = (device as { c8y_SupportedConfigurations?: unknown } | undefined)?.c8y_SupportedConfigurations;
  return Array.isArray(types) && types.includes(REGISTRATION_TYPE);
}

/** The connector file types a gateway offers, sorted. */
export function connectorTypes(device: unknown): string[] {
  const types = (device as { c8y_SupportedConfigurations?: unknown } | undefined)?.c8y_SupportedConfigurations;
  return Array.isArray(types) ? types.filter((t): t is string => isConnectorType(t)).sort() : [];
}

/** The point library types a gateway offers, sorted. */
export function libraryTypes(device: unknown): string[] {
  const types = (device as { c8y_SupportedConfigurations?: unknown } | undefined)?.c8y_SupportedConfigurations;
  return Array.isArray(types) ? types.filter((t): t is string => isLibraryType(t)).sort() : [];
}
