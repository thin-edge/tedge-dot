import { Injectable, inject } from '@angular/core';
import {
  EventBinaryService,
  IManagedObject,
  IOperation,
  IOperationBulk,
  InventoryBinaryService,
  InventoryService,
  OperationBulkService,
  OperationService,
} from '@c8y/client';
import { RepositoryService } from '@c8y/ngx-components/repository/shared';

import { parseToml } from '../generated/config-builder';
import { LibraryCatalog, packagedLibraries, referencedLibraries, referencedLibraryFiles } from './libraries';
import {
  type EntryLike,
  type ManagedFile,
  REGISTRATION_TYPE,
  entriesOfKind,
  fileOfType,
  libraryOf,
  newestByType,
} from './naming';
import { gatewaysQuery, groupsQuery, libraryEntriesQuery, managedEntriesQuery } from './queries';
import { type RegistrationSteps, registerFiles } from './register-flow';
import { waitForOperation } from './wait';

export { RegistrationCancelled } from './register-flow';

/** Asked before the plugin changes a gateway's list of managed files. */
export type ConfirmRegistration = (block: string) => Promise<boolean>;

/** Told what the plugin is doing, for a progress line. */
export type Progress = (step: string) => void;

/** A device in a group that lacks some of the types a send needs. */
export interface MissingTypes {
  device: IManagedObject;
  types: string[];
}

/**
 * Talks to Cumulocity on behalf of the OT connectors page and device tab: registering connector
 * files and point libraries on a gateway, and sending repository entries to gateways and groups.
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
  private readonly eventBinaries = inject(EventBinaryService);

  /** Whether the gateway offers the configuration type `type`. */
  offers(device: IManagedObject, type: string | undefined): boolean {
    const types: unknown = device['c8y_SupportedConfigurations'];
    return !!type && Array.isArray(types) && types.includes(type);
  }

  /**
   * Registers `files` in the gateway's tedge-configuration-plugin.toml, in one edit, and waits
   * until the gateway offers their types (see register-flow.ts). Resolves with the updated device.
   */
  async register(
    device: IManagedObject,
    files: readonly ManagedFile[],
    confirm: ConfirmRegistration,
    progress: Progress
  ): Promise<IManagedObject> {
    let current = device;
    const steps: RegistrationSteps = {
      missingTypes: async types => {
        current = (await this.inventory.detail(device.id)).data;
        return types.filter(type => !this.offers(current, type));
      },
      latestSnapshotId: async () => {
        const event = await this.repository.getLatestConfigurationEvent(device.id, REGISTRATION_TYPE);
        return event ? String(event.id) : undefined;
      },
      requestSnapshot: () => this.requestListSnapshot(device),
      latestSnapshot: () => this.latestListSnapshot(device),
      sendList: text => this.sendRegistrationList(device, text),
    };
    await registerFiles(steps, files, { confirm, progress });
    return current;
  }

  /** The text of a repository entry's file. */
  async entryText(entry: IManagedObject): Promise<string> {
    return entry['url'] ? this.repository.getBinaryText(entry['url'], { allowExternal: false }) : '';
  }

  /** The repository's point libraries: the newest entry of each library type, by name. */
  async libraryEntries(): Promise<IManagedObject[]> {
    const { data } = await this.inventory.list({ query: libraryEntriesQuery(), pageSize: 2000 });
    return [...newestByType(data as (IManagedObject & { configurationType?: string })[]).values()];
  }

  /**
   * The libraries a connector file may reference: the repository's, with their text, and the
   * packaged ones. A library whose text cannot be read still counts as existing.
   */
  async catalog(): Promise<LibraryCatalog> {
    const entries = await this.libraryEntries();
    const repository = await Promise.all(
      entries.map(async entry => {
        const { protocol, name } = libraryOf(entry['configurationType'])!;
        let config;
        try {
          config = parseToml(await this.entryText(entry));
        } catch {
          config = undefined;
        }
        return { protocol, name, source: 'repository' as const, config };
      })
    );
    return new LibraryCatalog([...packagedLibraries(), ...repository]);
  }

  /**
   * The repository libraries a connector entry references, in the order its devices name them.
   * A file that does not parse references none: the editor does not save such a file.
   */
  async librariesOf(entry: IManagedObject): Promise<IManagedObject[]> {
    let config;
    try {
      config = parseToml(await this.entryText(entry));
    } catch {
      return [];
    }
    const entries = await this.libraryEntries();
    const byType = new Map(entries.map(e => [e['configurationType'] as string, e]));
    return referencedLibraryFiles(config, byType.keys()).map(file => byType.get(file.type)!);
  }

  /**
   * The connector entries whose devices reference the library `name` of `protocol`. Reads every
   * connector entry's file, so it is meant for a single action such as deleting a library.
   */
  async connectorsReferencing(protocol: string, name: string): Promise<IManagedObject[]> {
    const { data } = await this.inventory.list({ query: managedEntriesQuery(), pageSize: 2000 });
    const referencing = await Promise.all(
      entriesOfKind(data as (IManagedObject & EntryLike)[], 'connector').map(async entry => {
        try {
          const config = parseToml(await this.entryText(entry));
          return config['connector']?.protocol === protocol && referencedLibraries(config).includes(name) ? entry : undefined;
        } catch {
          return undefined;
        }
      })
    );
    return referencing.filter((e): e is IManagedObject & EntryLike => !!e);
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

  /** Sends a repository entry to one gateway and waits until the gateway applied it. */
  async sendToGatewayAndWait(device: IManagedObject, entry: IManagedObject): Promise<void> {
    const created = await this.sendToGateway(device, entry);
    const id = required(created.id, 'operation');
    await waitForOperation(async () => (await this.operations.detail(id)).data, `sending ${entry['name']}`);
  }

  /** Sends a repository entry to every device in a group, as one bulk operation. */
  async sendToGroup(groupId: string | number, entry: IManagedObject): Promise<IOperationBulk> {
    const what = fileOfType(entry['configurationType'])?.kind === 'library' ? 'point library' : 'connector file';
    const bulk: IOperationBulk = {
      groupId,
      startDate: new Date().toISOString(),
      creationRamp: 1,
      operationPrototype: {
        description: `Send ${what} ${entry['name']} (${entry['configurationType']})`,
        c8y_DownloadConfigFile: { type: entry['configurationType'], url: entry['url'] },
      },
    };
    const { data } = await this.bulk.create(bulk);
    return data;
  }

  /**
   * The devices directly in a group, and those of them that do not offer every one of `types`,
   * with the types each lacks. Subgroups are not searched.
   */
  async checkGroup(groupId: string | number, types: readonly string[]): Promise<{ devices: number; missing: MissingTypes[] }> {
    const { data } = await this.inventory.childAssetsList(groupId, { pageSize: 2000 });
    const devices = data.filter(mo => mo['c8y_IsDevice'] !== undefined);
    const missing = devices
      .map(device => ({ device, types: types.filter(type => !this.offers(device, type)) }))
      .filter(m => m.types.length > 0);
    return { devices: devices.length, missing };
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

  /**
   * The newest snapshot of the list of managed files: the latest event of that type and its
   * attached file. RepositoryService.getConfigSnapshot drops the event id, which tells a new
   * snapshot from the previous one.
   */
  private async latestListSnapshot(device: IManagedObject): Promise<{ id: string; text: string } | undefined> {
    const event = await this.repository.getLatestConfigurationEvent(device.id, REGISTRATION_TYPE);
    if (!event) {
      return undefined;
    }
    const response = await this.eventBinaries.download(event);
    if (!response.ok) {
      throw new Error(`The snapshot of the list of managed files has no content (HTTP ${response.status}).`);
    }
    return { id: String(event.id), text: await response.text() };
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
