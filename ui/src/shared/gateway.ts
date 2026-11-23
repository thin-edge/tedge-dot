/**
 * Which devices get the OT connectors tab, and which members of a group a group send reaches.
 * Plain TypeScript so it is tested without Angular.
 */
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

/**
 * The members of a group that a bulk operation for the group reaches: its child assets, except
 * groups. A thin-edge.io child device counts although it has no c8y_IsDevice fragment; a bulk
 * operation sends to it all the same (seen on a tenant: the device got its operation).
 */
export function groupTargets<T extends object>(members: readonly T[]): T[] {
  return members.filter(member => {
    const mo = member as Record<string, unknown>;
    return mo['c8y_IsDeviceGroup'] === undefined && mo['c8y_IsDynamicGroup'] === undefined;
  });
}
