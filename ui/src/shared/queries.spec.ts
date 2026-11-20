import { connectorEntriesQuery, gatewaysQuery, groupsQuery, queryValue } from './queries';

describe('queries', () => {
  it('keeps a search term inside its quotes', () => {
    expect(queryValue("a' or name eq '*")).toBe('a or name eq *');
    expect(queryValue('a\\')).toBe('a');
  });

  it('lists connector entries by type prefix', () => {
    expect(connectorEntriesQuery()).toBe(
      "$filter=(type eq 'c8y_ConfigurationDump' and configurationType eq 'tedge-dot-*') $orderby=name"
    );
  });

  it('finds thin-edge.io gateways and groups by name', () => {
    expect(gatewaysQuery(' pump ')).toBe(
      "$filter=(has(c8y_IsDevice) and c8y_SupportedConfigurations eq 'tedge-configuration-plugin' and name eq '*pump*') $orderby=name"
    );
    expect(gatewaysQuery('')).toBe(
      "$filter=(has(c8y_IsDevice) and c8y_SupportedConfigurations eq 'tedge-configuration-plugin') $orderby=name"
    );
    expect(groupsQuery('line')).toBe("$filter=(has(c8y_IsDeviceGroup) and name eq '*line*') $orderby=name");
  });
});
