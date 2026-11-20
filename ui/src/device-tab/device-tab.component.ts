import { Component, OnInit, inject } from '@angular/core';
import { ActivatedRoute, Router } from '@angular/router';
import { IManagedObject, InventoryService } from '@c8y/client';
import { CoreModule, ResizableGridComponent, gettext } from '@c8y/ngx-components';
import {
  ConfigurationPreviewComponent,
  ConfigurationRepositoryDeviceTabModule,
  DeviceConfigurationListComponent,
} from '@c8y/ngx-components/repository/configuration';
import {
  ConfigurationSnapshot,
  DeviceConfigurationOperation,
  RepositoryService,
  SupportedConfigurationItem,
} from '@c8y/ngx-components/repository/shared';
import { BsModalService } from 'ngx-bootstrap/modal';

import { SendToGatewayComponent } from '../connectors/send-to-gateway.component';
import { connectorTypes } from '../shared/gateway';

/**
 * The OT connectors tab of a thin-edge.io gateway: Device Management's Configuration tab, limited
 * to connector files, plus adding a connector file and registering one that is already there.
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
      <button class="btn btn-link" type="button" (click)="register()">
        <i c8yIcon="link"></i>
        {{ 'Register a file on the gateway' | translate }}
      </button>
    </c8y-action-bar-item>
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="reload()">
        <i c8yIcon="refresh" [class.icon-spin]="reloading"></i>
        {{ 'Reload' | translate }}
      </button>
    </c8y-action-bar-item>

    <div class="card content-fullpage">
      <div class="card-header separator">
        <div class="card-title">{{ 'Connector files' | translate }}</div>
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
                [itemIcon]="'plug'"
                [items]="items"
                [emptyState]="deviceEmptyState"
                (configSelected)="onTypeSelected($event)"
              ></c8y-device-configuration-list>
            </div>
            <div class="card--grid__inner-scroll d-flex d-col flex-grow min-height-0 bg-component">
              <div class="card-block d-flex d-col flex-grow min-height-0">
                @if (configurationType; as type) {
                  <c8y-device-configuration-preview
                    class="d-flex d-col flex-grow min-height-0"
                    [actionButtonIcon]="'arrow-down'"
                    [device]="device"
                    [configurationType]="type"
                    [configSnapshot]="configSnapshot!"
                    [canSaveSnapshot]="true"
                    [operationToTrigger]="uploadConfig"
                    [actionButtonText]="'Get snapshot from device' | translate"
                  ></c8y-device-configuration-preview>
                } @else {
                  <fieldset class="c8y-fieldset p-l-16 p-r-16 p-b-16 flex-grow d-flex d-col min-height-0">
                    <legend>{{ 'Preview' | translate }}</legend>
                    <c8y-ui-empty-state
                      [icon]="'file-text'"
                      [title]="'No connector file selected.' | translate"
                      [subtitle]="'Select a connector file to preview.' | translate"
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
              @if (!configurationType) {
                <c8y-ui-empty-state
                  [icon]="'plug'"
                  [title]="'No connector file selected.' | translate"
                  [subtitle]="'Select a connector file above to see its repository entries.' | translate"
                  [horizontal]="true"
                ></c8y-ui-empty-state>
              } @else {
                <div class="p-r-16">
                  <c8y-device-configuration-list
                    [itemIcon]="'file-text'"
                    [items]="repositoryItems"
                    [emptyState]="repositoryEmptyState"
                    [isFilterEnabled]="true"
                    (configSelected)="onEntrySelected($event)"
                  ></c8y-device-configuration-list>
                </div>
              }
            </div>
            <div class="card--grid__inner-scroll d-flex d-col flex-grow min-height-0 bg-component">
              <div class="card-block d-flex d-col flex-grow min-height-0">
                @if (repositorySnapshot && configurationType) {
                  <c8y-device-configuration-preview
                    class="d-flex d-col flex-grow min-height-0"
                    [actionButtonIcon]="'arrow-up'"
                    [device]="device"
                    [configurationType]="configurationType"
                    [configSnapshot]="repositorySnapshot"
                    [operationToTrigger]="downloadConfig"
                    [actionButtonText]="'Send configuration to device' | translate"
                    [editableRepositoryMo]="selectedEntry!"
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

  readonly uploadConfig = DeviceConfigurationOperation.UPLOAD_CONFIG;
  readonly downloadConfig = DeviceConfigurationOperation.DOWNLOAD_CONFIG;

  device!: IManagedObject;
  items: SupportedConfigurationItem[] = [];
  configurationType?: string;
  configSnapshot?: ConfigurationSnapshot;
  repositoryEntries: IManagedObject[] = [];
  repositoryItems: SupportedConfigurationItem[] = [];
  selectedEntry?: IManagedObject;
  repositorySnapshot?: ConfigurationSnapshot;
  reloading = false;

  readonly deviceEmptyState = {
    icon: 'plug',
    title: gettext('No connector files on this gateway.'),
    text: gettext('Add a connector file, or register one that is already on the gateway.'),
  };
  readonly repositoryEmptyState = {
    icon: 'plug',
    title: gettext('No repository entries for this file.'),
    text: gettext('Add one under Management > OT connectors, or save a snapshot to the repository.'),
  };

  ngOnInit(): void {
    this.device = this.route.snapshot.parent?.data['contextData'];
    this.items = connectorTypes(this.device).map(name => ({ name }));
  }

  /** The list emits the selected item, though its output is typed as a string. */
  async onTypeSelected(item: unknown): Promise<void> {
    this.configurationType = typeof item === 'string' ? item : (item as SupportedConfigurationItem).name;
    await this.updateSnapshots();
  }

  async onEntrySelected(item: unknown): Promise<void> {
    const id = typeof item === 'string' ? undefined : (item as IManagedObject).id;
    const entry = this.repositoryEntries.find(e => e.id === id || e['name'] === item);
    if (!entry) {
      return;
    }
    this.selectedEntry = entry;
    this.repositorySnapshot = {
      id: String(entry.id),
      time: entry['creationTime'],
      name: entry['name'],
      binaryUrl: entry['url'],
    };
    if (entry['url']) {
      try {
        const text = await this.repository.getBinaryText(entry['url'], { allowExternal: false });
        this.repositorySnapshot = { ...this.repositorySnapshot, binary: text };
      } catch {
        // The preview shows the entry without content.
      }
    }
  }

  editEntry(entry: IManagedObject): void {
    void this.router.navigate(['/ot-connectors', entry.id]);
  }

  async reload(): Promise<void> {
    this.reloading = true;
    try {
      const { data } = await this.inventory.detail(this.device.id);
      this.device = data;
      this.items = connectorTypes(data).map(name => ({ name }));
      if (this.configurationType) {
        await this.updateSnapshots();
      }
    } finally {
      this.reloading = false;
    }
  }

  add(): void {
    this.openDialog(false);
  }

  register(): void {
    this.openDialog(true);
  }

  private openDialog(registerOnly: boolean): void {
    this.bsModal.show(SendToGatewayComponent, {
      initialState: { device: this.device, registerOnly, onFinished: () => void this.reload() },
      class: 'modal-lg',
      ignoreBackdropClick: true,
    });
  }

  private async updateSnapshots(): Promise<void> {
    if (!this.configurationType) {
      return;
    }
    this.selectedEntry = undefined;
    this.repositorySnapshot = undefined;
    this.repositoryEntries = await this.repository.getSnapshotsFromRepository(this.device, this.configurationType);
    this.repositoryItems = this.repositoryEntries.map(e => ({ ...e, name: e['name'] ?? '' }));
    this.configSnapshot = await this.repository.getConfigSnapshot(this.device, this.configurationType);
  }
}
