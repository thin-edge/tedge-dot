import { CanActivateFn } from '@angular/router';

import { isThinEdgeGateway } from '../shared/gateway';

/**
 * Shows the OT connectors tab on thin-edge.io gateways only. The tabs resolver runs the guard
 * with the device route, where the device is in `data`; navigation runs it with the child route,
 * where it is in the parent's `data`.
 */
export const thinEdgeGatewayGuard: CanActivateFn = route =>
  isThinEdgeGateway(route.data?.['contextData'] ?? route.parent?.data?.['contextData']);
