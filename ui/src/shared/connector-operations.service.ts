import { Injectable, inject } from '@angular/core';
import {
  IManagedObject,
  IOperation,
  IOperationBulk,
  InventoryBinaryService,
  InventoryService,
  OperationBulkService,
  OperationService,
} from '@c8y/client';
import { RepositoryService } from '@c8y/ngx-components/repository/shared';

import { REGISTRATION_TYPE, typeOf } from './naming';
import { gatewaysQuery, groupsQuery } from './queries';
import { type RegistrationSteps, registerConnectorFile } from './register-flow';
import { waitForOperation } from './wait';

export { RegistrationCancelled } from './register-flow';

/** Asked before the plugin changes a gateway's list of managed files. */
export type ConfirmRegistration = (block: string) => Promise<boolean>;

/** Told what the plugin is doing, for a progress line. */
export type Progress = (step: string) => void;

/**
 * Talks to Cumulocity on behalf of the OT connectors page and device tab: registering a
 * connector file on a gateway, and sending repository entries to gateways and groups.
 *
 * Provided by the components that use it, next to RepositoryService, which the platform does not
 * provide at the root.
 */
@Injectable()
export class ConnectorOperationsService {
  private readonly repository = inject(RepositoryService);
  private readonly operations = inject(OperationService);
  private readonly inventory = inject(InventoryService);
  private readonly binaries = inject(InventoryBinaryService);
  private readonly bulk = inject(OperationBulkService);

  /** Whether the gateway offers the connector file `name` as a configuration type. */
  offers(device: IManagedObject, name: string): boolean {
    const types: unknown = device['c8y_SupportedConfigurations'];
    return Array.isArray(types) && types.includes(typeOf(name));
  }

  /**
   * Registers the connector file `name` in the gateway's tedge-configuration-plugin.toml and waits
   * until the gateway offers its type (see register-flow.ts). Resolves with the updated device.
   */
  async register(
    device: IManagedObject,
    name: string,
    confirm: ConfirmRegistration,
    progress: Progress
  ): Promise<IManagedObject> {
    let current = device;
    const steps: RegistrationSteps = {
      offersType: async () => {
        current = (await this.inventory.detail(device.id)).data;
        return this.offers(current, name);
      },
      latestSnapshotId: async () => (await this.repository.getConfigSnapshot(device, REGISTRATION_TYPE))?.id,
      requestSnapshot: () => this.requestListSnapshot(device),
      latestSnapshot: () => this.latestListSnapshot(device),
      sendList: text => this.sendRegistrationList(device, text),
    };
    await registerConnectorFile(steps, name, { confirm, progress });
    return current;
  }

  /** Sends a repository entry to one gateway. */
  async sendToGateway(device: IManagedObject, entry: IManagedObject): Promise<IOperation> {
    const operation = this.repository.getDownloadConfigurationFileOperation(device, entry['configurationType'], {
      name: entry['name'],
      time: entry['lastUpdated'],
      binaryUrl: entry['url'],
    });
    const { data } = await this.operations.create(operation);
    return data;
  }

  /** Sends a repository entry to every device in a group, as one bulk operation. */
  async sendToGroup(groupId: string | number, entry: IManagedObject): Promise<IOperationBulk> {
    const bulk: IOperationBulk = {
      groupId,
      startDate: new Date().toISOString(),
      creationRamp: 1,
      operationPrototype: {
        description: `Send connector file ${entry['name']} (${entry['configurationType']})`,
        c8y_DownloadConfigFile: { type: entry['configurationType'], url: entry['url'] },
      },
    };
    const { data } = await this.bulk.create(bulk);
    return data;
  }

  /**
   * The devices directly in a group, and those of them that do not offer the connector file
   * `name`. Subgroups are not searched.
   */
  async checkGroup(groupId: string | number, name: string): Promise<{ devices: number; missing: IManagedObject[] }> {
    const { data } = await this.inventory.childAssetsList(groupId, { pageSize: 2000 });
    const devices = data.filter(mo => mo['c8y_IsDevice'] !== undefined);
    return { devices: devices.length, missing: devices.filter(mo => !this.offers(mo, name)) };
  }

  /** thin-edge.io gateways whose name contains `term`. */
  async findGateways(term: string): Promise<IManagedObject[]> {
    const { data } = await this.inventory.list({ query: gatewaysQuery(term), pageSize: 20 });
    return data;
  }

  /** Groups whose name contains `term`. */
  async findGroups(term: string): Promise<IManagedObject[]> {
    const { data } = await this.inventory.list({ query: groupsQuery(term), pageSize: 20 });
    return data;
  }

  /** Asks the gateway for a snapshot of its list of managed files and waits for the operation. */
  private async requestListSnapshot(device: IManagedObject): Promise<void> {
    const request = this.repository.getUploadConfigurationFileOperation(device, REGISTRATION_TYPE);
    const { data: created } = await this.operations.create(request);
    const requestId = required(created.id, 'operation');
    await waitForOperation(async () => (await this.operations.detail(requestId)).data, 'Getting the list of managed files');
  }

  /** The newest snapshot of the gateway's list of managed files, with its text. */
  private async latestListSnapshot(device: IManagedObject): Promise<{ id?: string; text: string } | undefined> {
    const snapshot = await this.repository.getConfigSnapshot(device, REGISTRATION_TYPE);
    if (!snapshot) {
      return undefined;
    }
    if (snapshot.binaryUrl) {
      return { id: snapshot.id, text: await this.repository.getBinaryText(snapshot.binaryUrl, { allowExternal: false }) };
    }
    if (typeof snapshot.binary === 'string') {
      return { id: snapshot.id, text: snapshot.binary };
    }
    throw new Error('The snapshot of the list of managed files has no content.');
  }

  /** Uploads the new list, sends it to the gateway, waits for the result, and removes the upload. */
  private async sendRegistrationList(device: IManagedObject, text: string): Promise<void> {
    const file = new File([text], 'tedge-configuration-plugin.toml', { type: 'text/plain' });
    const { data: binary } = await this.binaries.create(file);
    const binaryId = required(binary.id, 'upload');
    try {
      const operation = this.repository.getDownloadConfigurationFileOperation(device, REGISTRATION_TYPE, {
        name: 'tedge-configuration-plugin.toml',
        time: new Date().toISOString(),
        binaryUrl: binary.self,
      });
      const { data: created } = await this.operations.create(operation);
      const sendId = required(created.id, 'operation');
      await waitForOperation(async () => (await this.operations.detail(sendId)).data, 'Sending the list of managed files');
    } finally {
      await this.binaries.delete(binaryId).catch(() => undefined);
    }
  }
}

/** An id the platform always returns for something it created. */
function required(id: string | number | undefined, what: string): string | number {
  if (id === undefined || id === null) {
    throw new Error(`Cumulocity returned no id for the ${what}.`);
  }
  return id;
}
