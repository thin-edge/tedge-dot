/**
 * Which actions the plugin offers, by the user's global roles. It hides only what Device
 * Management hides: adding repository entries. Everything else stays visible, as in Device
 * Management, and the platform refuses what the user may not do. Inventory roles can allow
 * sending to some devices, which global roles cannot tell.
 */

/** Whether the user has any of these global roles (Permissions.hasAnyRole in the app). */
export type HasAnyRole = (roles: string[]) => boolean;

/**
 * Whether the user can add a repository entry: an object and its file. This is the rule Device
 * Management uses for "Save to repository" on a configuration snapshot.
 */
export function canAddEntries(hasAnyRole: HasAnyRole): boolean {
  return (
    hasAnyRole(['ROLE_INVENTORY_ADMIN', 'ROLE_INVENTORY_CREATE']) ||
    (hasAnyRole(['ROLE_MANAGED_OBJECT_ADMIN', 'ROLE_MANAGED_OBJECT_CREATE']) &&
      hasAnyRole(['ROLE_BINARY_ADMIN', 'ROLE_BINARY_CREATE']))
  );
}
