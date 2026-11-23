import { connectorTypes, groupTargets, isThinEdgeGateway } from './gateway';

describe('gateway', () => {
  const gateway = {
    c8y_SupportedConfigurations: ['tedge.toml', 'tedge-configuration-plugin', 'tedge-dot-zeta', 'tedge-dot-alpha'],
  };

  it('recognises thin-edge.io gateways', () => {
    expect(isThinEdgeGateway(gateway)).toBe(true);
    expect(isThinEdgeGateway({ c8y_SupportedConfigurations: ['agent.conf'] })).toBe(false);
    expect(isThinEdgeGateway({})).toBe(false);
    expect(isThinEdgeGateway(undefined)).toBe(false);
  });

  it('lists the connector types a gateway offers', () => {
    expect(connectorTypes(gateway)).toEqual(['tedge-dot-alpha', 'tedge-dot-zeta']);
    expect(connectorTypes({})).toEqual([]);
  });

  it('counts every member of a group a bulk operation reaches, but no groups', () => {
    const gatewayDevice = { id: '1', c8y_IsDevice: {}, com_cumulocity_model_Agent: {} };
    // A thin-edge.io child device, as the mapper creates it: no c8y_IsDevice fragment.
    const childDevice = { id: '2', type: 'thin-edge.io-child', c8y_SupportedOperations: ['c8y_DownloadConfigFile'] };
    const subgroup = { id: '3', c8y_IsDeviceGroup: {} };
    const smartGroup = { id: '4', c8y_IsDynamicGroup: {} };
    expect(groupTargets([gatewayDevice, childDevice, subgroup, smartGroup]).map(m => m.id)).toEqual(['1', '2']);
    expect(groupTargets([])).toEqual([]);
  });
});
