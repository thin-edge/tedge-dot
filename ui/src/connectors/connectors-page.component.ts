import { Component, OnInit, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { IManagedObject, InventoryService } from '@c8y/client';
import { AlertService, CoreModule, ModalService, Status } from '@c8y/ngx-components';
import { gettext } from '@c8y/ngx-components/gettext';
import { RepositoryService, SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalService } from 'ngx-bootstrap/modal';

import { ConnectorOperationsService } from '../shared/connector-operations.service';
import { nameOf } from '../shared/naming';
import { connectorEntriesQuery } from '../shared/queries';
import { SendToGatewayComponent } from './send-to-gateway.component';
import { SendToGroupComponent } from './send-to-group.component';
import { UploadFilesComponent } from './upload-files.component';

/** Management > OT connectors: the configuration repository's connector files. */
@Component({
  selector: 'tdot-connectors-page',
  imports: [CoreModule, SharedRepositoryModule],
  providers: [ConnectorOperationsService],
  template: `
    <c8y-title>{{ 'OT connectors' | translate }}</c8y-title>
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="upload()">
        <i c8yIcon="upload"></i>
        {{ 'Upload connector files' | translate }}
      </button>
    </c8y-action-bar-item>
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="add()">
        <i c8yIcon="plus-circle"></i>
        {{ 'Add connector file' | translate }}
      </button>
    </c8y-action-bar-item>
    <c8y-action-bar-item [placement]="'right'">
      <button class="btn btn-link" type="button" (click)="load()">
        <i c8yIcon="refresh" [class.icon-spin]="loading()"></i>
        {{ 'Reload' | translate }}
      </button>
    </c8y-action-bar-item>

    <div class="card content-fullpage">
      <div class="card-header separator">
        <div class="card-title">{{ 'Connector files' | translate }}</div>
      </div>
      @if (!loading() && entries().length === 0) {
        <c8y-ui-empty-state
          [icon]="'gears'"
          [title]="'No connector files in the repository.' | translate"
          [subtitle]="'Add one, or save a snapshot from a gateway.' | translate"
          [horizontal]="true"
        ></c8y-ui-empty-state>
      } @else {
        <div class="table-responsive">
          <table class="table large-padding">
            <thead>
              <tr>
                <th>{{ 'Name' | translate }}</th>
                <th>{{ 'Description' | translate }}</th>
                <th>{{ 'File' | translate }}</th>
                <th>{{ 'Device type' | translate }}</th>
                <th>{{ 'Last updated' | translate }}</th>
                <th class="text-right">{{ 'Actions' | translate }}</th>
              </tr>
            </thead>
            <tbody>
              @for (entry of entries(); track entry.id) {
                <tr>
                  <td><a class="interact" (click)="edit(entry)">{{ entry['name'] }}</a></td>
                  <td>{{ entry['description'] }}</td>
                  <td>{{ fileOf(entry) }}</td>
                  <td>{{ entry['deviceType'] }}</td>
                  <td>{{ entry['lastUpdated'] | c8yDate }}</td>
                  <td class="text-right text-nowrap">
                    <button class="btn btn-default btn-xs m-r-4" type="button" (click)="edit(entry)">
                      {{ 'Edit' | translate }}
                    </button>
                    <button class="btn btn-default btn-xs m-r-4" type="button" (click)="sendToGateway(entry)">
                      {{ 'Send to a gateway' | translate }}
                    </button>
                    <button class="btn btn-default btn-xs m-r-4" type="button" (click)="sendToGroup(entry)">
                      {{ 'Send to a group' | translate }}
                    </button>
                    <button class="btn btn-danger btn-xs" type="button" (click)="remove(entry)">
                      {{ 'Delete' | translate }}
                    </button>
                  </td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      }
    </div>
  `,
})
export class ConnectorsPageComponent implements OnInit {
  private readonly inventory = inject(InventoryService);
  private readonly repository = inject(RepositoryService);
  private readonly router = inject(Router);
  private readonly modal = inject(ModalService);
  private readonly bsModal = inject(BsModalService);
  private readonly alert = inject(AlertService);

  readonly entries = signal<IManagedObject[]>([]);
  readonly loading = signal(true);

  ngOnInit(): void {
    void this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    try {
      const { data } = await this.inventory.list({ query: connectorEntriesQuery(), pageSize: 2000 });
      this.entries.set(data);
    } catch (e) {
      this.alert.addServerFailure(e);
    } finally {
      this.loading.set(false);
    }
  }

  fileOf(entry: IManagedObject): string {
    const name = nameOf(entry['configurationType']);
    return name ? `${name}.toml` : '';
  }

  add(): void {
    void this.router.navigate(['/ot-connectors', 'new']);
  }

  edit(entry: IManagedObject): void {
    void this.router.navigate(['/ot-connectors', entry.id]);
  }

  sendToGateway(entry: IManagedObject): void {
    this.bsModal.show(SendToGatewayComponent, { initialState: { entry }, class: 'modal-lg', ignoreBackdropClick: true });
  }

  upload(): void {
    this.bsModal.show(UploadFilesComponent, {
      initialState: { onFinished: () => void this.load() },
      class: 'modal-lg',
      ignoreBackdropClick: true,
    });
  }

  sendToGroup(entry: IManagedObject): void {
    this.bsModal.show(SendToGroupComponent, { initialState: { entry }, class: 'modal-lg', ignoreBackdropClick: true });
  }

  async remove(entry: IManagedObject): Promise<void> {
    const confirmed = await this.modal
      .confirm(
        gettext('Delete connector file'),
        `${entry['name']}: the repository entry and its file are deleted. Gateways keep their copy.`,
        Status.DANGER,
        { ok: gettext('Delete'), cancel: gettext('Cancel') }
      )
      .catch(() => false);
    if (!confirmed) {
      return;
    }
    try {
      await this.repository.delete(entry);
      this.alert.success(gettext('Connector file deleted.'));
      await this.load();
    } catch (e) {
      this.alert.addServerFailure(e);
    }
  }
}
