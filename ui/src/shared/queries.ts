/**
 * Inventory queries the plugin sends. Plain strings in the platform's query language, built here
 * so they are tested without a tenant.
 */
import { REGISTRATION_TYPE, TYPE_PREFIX } from './naming';

/** A search term as a quoted query value: quotes and backslashes cannot end the string early. */
export function queryValue(term: string): string {
  return term.replace(/[\\']/g, '');
}

/** Repository entries of connector files, by name. */
export function connectorEntriesQuery(): string {
  return `$filter=(type eq 'c8y_ConfigurationDump' and configurationType eq '${TYPE_PREFIX}*') $orderby=name`;
}

/** thin-edge.io gateways whose name contains `term`. */
export function gatewaysQuery(term: string): string {
  const name = queryValue(term.trim());
  const byName = name ? ` and name eq '*${name}*'` : '';
  return `$filter=(has(c8y_IsDevice) and c8y_SupportedConfigurations eq '${REGISTRATION_TYPE}'${byName}) $orderby=name`;
}

/** Groups whose name contains `term`. */
export function groupsQuery(term: string): string {
  const name = queryValue(term.trim());
  const byName = name ? ` and name eq '*${name}*'` : '';
  return `$filter=(has(c8y_IsDeviceGroup)${byName}) $orderby=name`;
}
