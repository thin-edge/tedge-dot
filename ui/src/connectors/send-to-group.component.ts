import { Component, inject } from '@angular/core';
import { IManagedObject } from '@c8y/client';
import { CoreModule } from '@c8y/ngx-components';
import { SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { ConnectorOperationsService } from '../shared/connector-operations.service';
import { nameOf } from '../shared/naming';

type State = 'pick' | 'checked' | 'running' | 'done' | 'error';

/**
 * Sends a connector file to every device in a group, as one bulk operation. A group send never
 * registers files: gateways that do not offer the type are listed first, because their
 * operations would fail.
 */
@Component({
  selector: 'tdot-send-to-group',
  imports: [CoreModule, SharedRepositoryModule],
  providers: [ConnectorOperationsService],
  template: `
    <div class="modal-header dialog-header">
      <i c8yIcon="c8y-group"></i>
      <h4>{{ 'Send connector file to a group' | translate }}</h4>
    </div>
    <div class="modal-body">
      <p>{{ 'Connector file' | translate }}: <strong>{{ entry?.['name'] }}</strong> · {{ name }}.toml</p>
      @switch (state) {
        @case ('pick') {
          <c8y-form-group>
            <label for="tdot-group-search">{{ 'Group' | translate }}</label>
            <input id="tdot-group-search" class="form-control" [(ngModel)]="term" (ngModelChange)="search()"
              [placeholder]="'Search groups by name' | translate" />
          </c8y-form-group>
          <div class="list-group">
            @for (g of groups; track g.id) {
              <button type="button" class="list-group-item" [class.active]="picked?.id === g.id" (click)="picked = g">
                {{ g['name'] }}
              </button>
            } @empty {
              <p class="text-muted">{{ 'No group matches.' | translate }}</p>
            }
          </div>
        }
        @case ('checked') {
          @if (missing.length) {
            <p class="text-warning">
              <i c8yIcon="exclamation-triangle"></i>
              {{ missing.length }} {{ 'devices in this group do not offer this file, so their operations will fail. Add the file to each of them from its OT connectors tab first:' | translate }}
            </p>
            <ul>
              @for (d of missing; track d.id) {
                <li>{{ d['name'] }}</li>
              }
            </ul>
          } @else {
            <p>{{ 'Every device in this group offers this file.' | translate }}</p>
          }
        }
        @case ('running') {
          <p><i c8yIcon="spinner" class="icon-spin"></i> {{ 'Creating the bulk operation' | translate }}…</p>
        }
        @case ('done') {
          <p class="text-success"><i c8yIcon="check-circle"></i> {{ doneText }}</p>
        }
        @case ('error') {
          <p class="text-danger"><i c8yIcon="exclamation-circle"></i> {{ errorText }}</p>
        }
      }
    </div>
    <div class="modal-footer">
      @switch (state) {
        @case ('pick') {
          <button class="btn btn-default" type="button" (click)="close()">{{ 'Cancel' | translate }}</button>
          <button class="btn btn-primary" type="button" [disabled]="!picked" (click)="check()">{{ 'Next' | translate }}</button>
        }
        @case ('checked') {
          <button class="btn btn-default" type="button" (click)="state = 'pick'">{{ 'Back' | translate }}</button>
          <button class="btn btn-primary" type="button" (click)="send()">{{ 'Send to the group' | translate }}</button>
        }
        @case ('running') {
          <button class="btn btn-default" type="button" disabled>{{ 'Working' | translate }}</button>
        }
        @default {
          <button class="btn btn-primary" type="button" (click)="close()">{{ 'Close' | translate }}</button>
        }
      }
    </div>
  `,
})
export class SendToGroupComponent {
  private readonly ops = inject(ConnectorOperationsService);
  private readonly modalRef = inject(BsModalRef);

  /** Set by the opener. */
  entry?: IManagedObject;

  state: State = 'pick';
  term = '';
  groups: IManagedObject[] = [];
  picked?: IManagedObject;
  missing: IManagedObject[] = [];
  doneText = '';
  errorText = '';

  get name(): string {
    return nameOf(this.entry?.['configurationType']) ?? '';
  }

  constructor() {
    void this.search();
  }

  async search(): Promise<void> {
    this.groups = await this.ops.findGroups(this.term);
  }

  async check(): Promise<void> {
    if (!this.picked) {
      return;
    }
    try {
      this.missing = await this.ops.groupDevicesWithout(this.picked.id, this.name);
      this.state = 'checked';
    } catch (e) {
      this.errorText = e instanceof Error ? e.message : String(e);
      this.state = 'error';
    }
  }

  async send(): Promise<void> {
    if (!this.picked || !this.entry) {
      return;
    }
    this.state = 'running';
    try {
      const bulk = await this.ops.sendToGroup(this.picked.id, this.entry);
      this.doneText = `Bulk operation ${bulk.id} sends ${this.entry['name']} to ${this.picked['name']}. Its progress is under Device control > Bulk operations.`;
      this.state = 'done';
    } catch (e) {
      this.errorText = e instanceof Error ? e.message : String(e);
      this.state = 'error';
    }
  }

  close(): void {
    this.modalRef.hide();
  }
}
