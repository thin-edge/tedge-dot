import { Component, DestroyRef, OnInit, computed, inject, signal, viewChild } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { ActivatedRoute, Router } from '@angular/router';
import { IManagedObject, IOperation, InventoryService, OperationService } from '@c8y/client';
import { CoreModule, OperationRealtimeService, Permissions, ResizableGridComponent } from '@c8y/ngx-components';
import { gettext } from '@c8y/ngx-components/gettext';
import {
  ConfigurationPreviewComponent,
  ConfigurationRepositoryDeviceTabModule,
  DeviceConfigurationListComponent,
  DeviceConfigurationService,
} from '@c8y/ngx-components/repository/configuration';
import {
  ConfigurationSnapshot,
  DeviceConfigurationOperation,
  RepositoryService,
  SupportedConfigurationItem,
} from '@c8y/ngx-components/repository/shared';
import { BsModalService } from 'ngx-bootstrap/modal';

import { SendToGatewayComponent } from '../connectors/send-to-gateway.component';
import { type Route, entryRoute, newFileRoute } from '../shared/editor-context';
import { connectorTypes, libraryTypes } from '../shared/gateway';
import { type ConfigOperation, missingOnGateway } from '../shared/missing-file';
import { fileOfType } from '../shared/naming';
import { canAddEntries } from '../shared/permissions';

/**
 * The OT connectors tab of a thin-edge.io gateway: Device Management's Configuration tab, limited
 * to connector files and, in a second list, point libraries. "Add connector file" sends a file
 * from the repository, creates a new one, or registers one that is already on the gateway; a
 * registered type without a repository entry can be created from here, and a registered file the
 * gateway does not have is pointed out.
 */
@Component({
  selector: 'tdot-device-tab',
  imports: [
    CoreModule,
    ConfigurationRepositoryDeviceTabModule,
    DeviceConfigurationListComponent,
    ConfigurationPreviewComponent,
    ResizableGridComponent,
  ],
  template: `
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="add()">
        <i c8yIcon="plus-circle"></i>
        {{ 'Add connector file' | translate }}
      </button>
    </c8y-action-bar-item>
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="reload()">
        <i c8yIcon="refresh" [class.icon-spin]="reloading()"></i>
        {{ 'Reload' | translate }}
      </button>
    </c8y-action-bar-item>

    <div class="card content-fullpage">
      <div class="card-header separator">
        <div class="card-title">{{ 'Connector files and point libraries' | translate }}</div>
      </div>
      <c8y-resizable-grid
        [orientation]="'vertical'"
        [trackId]="'tdot-device-ot-connectors-split-view'"
        [leftColumnWidth]="'50%'"
        [collapsible]="false"
      >
        <div class="d-flex d-col fit-h min-height-0" left-pane>
          <div class="p-l-16 p-r-16">
            <h5 class="legend form-block"><span>{{ 'On device' | translate }}</span></h5>
          </div>
          <div class="card--grid grid__col--4-8--md m-b-0 flex-grow min-height-0">
            <div class="card--grid__inner-scroll bg-level-1">
              <c8y-device-configuration-list
                #connectorList
                [itemIcon]="'plug'"
                [items]="items()"
                [emptyState]="deviceEmptyState"
                (configSelected)="onTypeSelected($event, 'connector')"
              ></c8y-device-configuration-list>
              @if (libraryItems().length) {
                <p class="text-label-small text-muted p-l-16 p-t-16 m-b-0">{{ 'Point libraries' | translate }}</p>
                <c8y-device-configuration-list
                  #libraryList
                  [itemIcon]="'list'"
                  [items]="libraryItems()"
                  [emptyState]="deviceEmptyState"
                  (configSelected)="onTypeSelected($event, 'library')"
                ></c8y-device-configuration-list>
              }
            </div>
            <div class="card--grid__inner-scroll d-flex d-col flex-grow min-height-0 bg-component">
              <div class="card-block d-flex d-col flex-grow min-height-0">
                @if (missingPath(); as path) {
                  <div class="alert alert-warning m-b-16" role="status">
                    <p class="m-b-4"><strong>{{ 'Registered, but missing on the gateway' | translate }}</strong></p>
                    <p class="m-b-0">
                      {{ 'The gateway lists this file, but its last snapshot request found no file at' | translate }}
                      <code>{{ path }}</code>.
                      {{ (repositoryItems().length ? 'Send a repository entry to put it there.' : canAdd ? 'Create it, and send it to put it there.' : 'Add a repository entry for it under Management > OT connectors, then send it.') | translate }}
                    </p>
                    @if (canCreate()) {
                      <button class="btn btn-default btn-sm m-t-8" type="button" (click)="create()">{{ createLabel() | translate }}</button>
                    }
                  </div>
                }
                @if (configurationType(); as type) {
                  <c8y-device-configuration-preview
                    class="d-flex d-col flex-grow min-height-0"
                    [actionButtonIcon]="'arrow-down'"
                    [device]="device()"
                    [configurationType]="type"
                    [configSnapshot]="configSnapshot()!"
                    [canSaveSnapshot]="true"
                    [operationToTrigger]="uploadConfig"
                    [actionButtonText]="'Get snapshot from device' | translate"
                  ></c8y-device-configuration-preview>
                } @else {
                  <fieldset class="c8y-fieldset p-l-16 p-r-16 p-b-16 flex-grow d-flex d-col min-height-0">
                    <legend>{{ 'Preview' | translate }}</legend>
                    <c8y-ui-empty-state
                      [icon]="'file-text'"
                      [title]="'No file selected.' | translate"
                      [subtitle]="'Select a connector file or point library to preview.' | translate"
                      [horizontal]="true"
                    ></c8y-ui-empty-state>
                  </fieldset>
                }
              </div>
            </div>
          </div>
        </div>

        <div class="d-flex d-col fit-h min-height-0" right-pane>
          <div class="p-l-16 p-r-16">
            <h5 class="legend form-block"><span>{{ 'Available from repository' | translate }}</span></h5>
          </div>
          <div class="card--grid grid__col--4-8--md m-b-0 flex-grow min-height-0">
            <div class="card--grid__inner-scroll bg-level-1">
              @if (!configurationType()) {
                <c8y-ui-empty-state
                  [icon]="'plug'"
                  [title]="'No file selected.' | translate"
                  [subtitle]="'Select a connector file or point library to see its repository entries.' | translate"
                  [horizontal]="true"
                ></c8y-ui-empty-state>
              } @else if (repositoryLoaded() && !repositoryItems().length) {
                <c8y-ui-empty-state
                  [icon]="'plug'"
                  [title]="'No repository entries for this file.' | translate"
                  [subtitle]="(canAdd ? 'Create one, and send it to this gateway.' : 'Add one under Management > OT connectors, or save a snapshot to the repository.') | translate"
                  [horizontal]="true"
                >
                  @if (canCreate()) {
                    <button class="btn btn-default btn-sm m-t-8" type="button" (click)="create()">{{ createLabel() | translate }}</button>
                  }
                </c8y-ui-empty-state>
              } @else {
                <div class="p-r-16">
                  <c8y-device-configuration-list
                    [itemIcon]="'file-text'"
                    [items]="repositoryItems()"
                    [emptyState]="repositoryEmptyState"
                    [isFilterEnabled]="true"
                    (configSelected)="onEntrySelected($event)"
                  ></c8y-device-configuration-list>
                </div>
              }
            </div>
            <div class="card--grid__inner-scroll d-flex d-col flex-grow min-height-0 bg-component">
              <div class="card-block d-flex d-col flex-grow min-height-0">
                @if (repositorySnapshot(); as snapshot) {
                  <c8y-device-configuration-preview
                    class="d-flex d-col flex-grow min-height-0"
                    [actionButtonIcon]="'arrow-up'"
                    [device]="device()"
                    [configurationType]="configurationType()!"
                    [configSnapshot]="snapshot"
                    [operationToTrigger]="downloadConfig"
                    [actionButtonText]="'Send configuration to device' | translate"
                    [editableRepositoryMo]="selectedEntry()!"
                    (editInRepository)="editEntry($event)"
                  ></c8y-device-configuration-preview>
                } @else {
                  <fieldset class="c8y-fieldset p-l-16 p-r-16 p-b-16 flex-grow d-flex d-col min-height-0">
                    <legend>{{ 'Preview' | translate }}</legend>
                    <c8y-ui-empty-state
                      [icon]="'file-text'"
                      [title]="'No repository entry selected.' | translate"
                      [subtitle]="'Select an entry to preview.' | translate"
                      [horizontal]="true"
                    ></c8y-ui-empty-state>
                  </fieldset>
                }
              </div>
            </div>
          </div>
        </div>
      </c8y-resizable-grid>
    </div>
  `,
})
export class DeviceTabComponent implements OnInit {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly repository = inject(RepositoryService);
  private readonly inventory = inject(InventoryService);
  private readonly bsModal = inject(BsModalService);
  private readonly configurations = inject(DeviceConfigurationService);
  private readonly operations = inject(OperationService);
  private readonly realtime = inject(OperationRealtimeService, { optional: true });
  private readonly permissions = inject(Permissions);
  private readonly destroyRef = inject(DestroyRef);

  readonly uploadConfig = DeviceConfigurationOperation.UPLOAD_CONFIG;
  readonly downloadConfig = DeviceConfigurationOperation.DOWNLOAD_CONFIG;

  readonly device = signal<IManagedObject>({} as IManagedObject);
  readonly items = signal<SupportedConfigurationItem[]>([]);
  readonly libraryItems = signal<SupportedConfigurationItem[]>([]);
  private readonly connectorList = viewChild<DeviceConfigurationListComponent>('connectorList');
  private readonly libraryList = viewChild<DeviceConfigurationListComponent>('libraryList');
  readonly configurationType = signal<string | undefined>(undefined);
  readonly configSnapshot = signal<ConfigurationSnapshot | undefined>(undefined);
  readonly repositoryItems = signal<SupportedConfigurationItem[]>([]);
  readonly selectedEntry = signal<IManagedObject | undefined>(undefined);
  readonly repositorySnapshot = signal<ConfigurationSnapshot | undefined>(undefined);
  readonly reloading = signal(false);
  /** Whether the repository entries of the selected type are loaded. */
  readonly repositoryLoaded = signal(false);
  /** Whether the gateway lists the selected file but does not have it. */
  readonly missing = signal(false);
  private repositoryEntries: IManagedObject[] = [];

  /** Whether the user can add repository entries; without it, nothing is created from here. */
  readonly canAdd = canAddEntries(roles => this.permissions.hasAnyRole(roles));
  readonly selectedFile = computed(() => fileOfType(this.configurationType()));
  readonly missingPath = computed(() => (this.missing() ? this.selectedFile()?.path : undefined));
  readonly canCreate = computed(() => this.canAdd && !!this.selectedFile() && this.repositoryLoaded() && !this.repositoryItems().length);
  readonly createLabel = computed(() => (this.selectedFile()?.kind === 'library' ? 'Create point library' : 'Create connector file'));

  readonly deviceEmptyState = {
    icon: 'plug',
    title: gettext('No connector files on this gateway.'),
    text: gettext('Add a connector file: from the repository, as a new file, or one that is already on the gateway.'),
  };
  readonly repositoryEmptyState = {
    icon: 'plug',
    title: gettext('No repository entries for this file.'),
    text: gettext('Add one under Management > OT connectors, or save a snapshot to the repository.'),
  };

  constructor() {
    // The previews report a finished snapshot upload, and "Save to repository" (repository entries
    // only), the way they do in Device Management's Configuration tab.
    this.configurations.configurationsUpdated
      .pipe(takeUntilDestroyed())
      .subscribe((repositoryOnly?: boolean) => {
        void this.updateSnapshots(repositoryOnly);
        void this.checkMissing();
      });
  }

  ngOnInit(): void {
    const device: IManagedObject = this.route.snapshot.parent?.data['contextData'];
    this.device.set(device);
    this.showTypes(device);
    // A snapshot request or a send of the selected type may change whether the file is missing.
    this.realtime
      ?.onAll$(device.id)
      .pipe(takeUntilDestroyed(this.destroyRef))
      .subscribe(({ data }) => {
        const operation = data as ConfigOperation;
        const type = this.configurationType();
        if (type && (operation.c8y_UploadConfigFile?.type === type || operation.c8y_DownloadConfigFile?.type === type)) {
          void this.checkMissing();
        }
      });
  }

  /** The list emits the selected item, though its output is typed as a string. */
  async onTypeSelected(item: unknown, list: 'connector' | 'library'): Promise<void> {
    // One selection across both lists.
    const other = list === 'connector' ? this.libraryList() : this.connectorList();
    if (other) {
      other.selectedConfig = undefined;
    }
    this.configurationType.set(typeof item === 'string' ? item : (item as SupportedConfigurationItem).name);
    this.configSnapshot.set(undefined);
    this.missing.set(false);
    await Promise.all([this.updateSnapshots(), this.checkMissing()]);
  }

  async onEntrySelected(item: unknown): Promise<void> {
    const id = typeof item === 'string' ? undefined : (item as IManagedObject).id;
    const entry = this.repositoryEntries.find(e => e.id === id || e['name'] === item);
    if (!entry) {
      return;
    }
    this.selectedEntry.set(entry);
    const snapshot: ConfigurationSnapshot = {
      id: String(entry.id),
      time: entry['creationTime'],
      name: entry['name'],
      binaryUrl: entry['url'],
    };
    this.repositorySnapshot.set(snapshot);
    if (entry['url']) {
      try {
        const text = await this.repository.getBinaryText(entry['url'], { allowExternal: false });
        if (this.selectedEntry()?.id === entry.id) {
          this.repositorySnapshot.set({ ...snapshot, binary: text });
        }
      } catch {
        // The preview shows the entry without content.
      }
    }
  }

  /** The entry in the editor, which returns to this tab and offers to send it here. */
  editEntry(entry: IManagedObject): void {
    this.navigate(entryRoute(entry.id, String(this.device().id)));
  }

  /** A new entry for the selected type, in the editor with its file name (and protocol) fixed. */
  create(): void {
    const file = this.selectedFile();
    if (file) {
      this.navigate(newFileRoute(file.kind, String(this.device().id), file));
    }
  }

  async reload(): Promise<void> {
    this.reloading.set(true);
    try {
      const { data } = await this.inventory.detail(this.device().id);
      this.device.set(data);
      this.showTypes(data);
      if (this.configurationType()) {
        await Promise.all([this.updateSnapshots(), this.checkMissing()]);
      }
    } finally {
      this.reloading.set(false);
    }
  }

  add(): void {
    this.bsModal.show(SendToGatewayComponent, {
      initialState: { device: this.device(), onFinished: () => void this.reload() },
      class: 'modal-lg',
      ignoreBackdropClick: true,
    });
  }

  /** The connector file types and the point library types the gateway offers. */
  private showTypes(device: IManagedObject): void {
    this.items.set(connectorTypes(device).map(name => ({ name })));
    this.libraryItems.set(libraryTypes(device).map(name => ({ name })));
  }

  private navigate(route: Route): void {
    void this.router.navigate(route.commands, { queryParams: route.queryParams });
  }

  /**
   * Whether the gateway lists the selected file but does not have it, from the newest snapshot
   * requests and sends of the gateway. The API cannot filter operations by configuration type.
   */
  private async checkMissing(): Promise<void> {
    const type = this.configurationType();
    if (!type) {
      return;
    }
    const deviceId = this.device().id;
    const newest = (fragmentType: string) =>
      this.operations.list({ deviceId, fragmentType, dateFrom: '1970-01-01T00:00:00.000Z', revert: true, pageSize: 50 });
    try {
      const [uploads, downloads] = await Promise.all([newest('c8y_UploadConfigFile'), newest('c8y_DownloadConfigFile')]);
      if (this.configurationType() === type) {
        const operations: IOperation[] = [...uploads.data, ...downloads.data];
        this.missing.set(missingOnGateway(operations as ConfigOperation[], type));
      }
    } catch {
      // Without operations, the tab shows no hint; the preview still shows the request's state.
      this.missing.set(false);
    }
  }

  private async updateSnapshots(repositoryOnly = false): Promise<void> {
    const type = this.configurationType();
    if (!type) {
      return;
    }
    const device = this.device();
    this.selectedEntry.set(undefined);
    this.repositorySnapshot.set(undefined);
    if (!repositoryOnly) {
      this.repositoryLoaded.set(false);
    }
    // The SDK also returns entries without a configuration type, which the Configuration tab
    // offers for any type. Here they would replace a connector file with unrelated content.
    const entries = await this.repository.getSnapshotsFromRepository(device, type);
    if (this.configurationType() !== type) {
      return;
    }
    this.repositoryEntries = entries.filter(e => e['configurationType'] === type);
    this.repositoryItems.set(this.repositoryEntries.map(e => ({ ...e, name: e['name'] ?? '' })));
    this.repositoryLoaded.set(true);
    if (repositoryOnly) {
      return;
    }
    const snapshot = await this.repository.getConfigSnapshot(device, type);
    if (this.configurationType() === type) {
      this.configSnapshot.set(snapshot);
    }
  }
}
