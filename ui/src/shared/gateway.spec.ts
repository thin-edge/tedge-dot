import { connectorTypes, isThinEdgeGateway } from './gateway';

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
});
