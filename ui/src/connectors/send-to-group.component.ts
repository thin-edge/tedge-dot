import { Component, inject, signal } from '@angular/core';
import { IManagedObject } from '@c8y/client';
import { CoreModule } from '@c8y/ngx-components';
import { SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { ConnectorOperationsService, type MissingTypes } from '../shared/connector-operations.service';
import { type ManagedFile, fileOfType } from '../shared/naming';

type State = 'pick' | 'checked' | 'running' | 'done' | 'error';

/**
 * Sends a connector file or point library to every device in a group, as one bulk operation. A
 * group send never registers files: gateways that do not offer the type, or a repository library
 * the connector file references, are listed first, because their operations would fail or the
 * connector would miss points.
 */
@Component({
  selector: 'tdot-send-to-group',
  imports: [CoreModule, SharedRepositoryModule],
  providers: [ConnectorOperationsService],
  template: `
    <div class="modal-header dialog-header">
      <i c8yIcon="c8y-group"></i>
      <h4>{{ (isLibrary ? 'Send point library to a group' : 'Send connector file to a group') | translate }}</h4>
    </div>
    <div class="modal-body">
      <p>
        {{ (isLibrary ? 'Point library' : 'Connector file') | translate }}: <strong>{{ entry?.['name'] }}</strong> ·
        {{ file?.path }}
      </p>
      @switch (state()) {
        @case ('pick') {
          <c8y-form-group>
            <label for="tdot-group-search">{{ 'Group' | translate }}</label>
            <input id="tdot-group-search" class="form-control" [(ngModel)]="term" (ngModelChange)="search($event)"
              [placeholder]="'Search groups by name' | translate" />
          </c8y-form-group>
          <div class="list-group">
            @for (g of groups(); track g.id) {
              <button type="button" class="list-group-item" [class.active]="picked()?.id === g.id" (click)="picked.set(g)">
                {{ g['name'] }}
              </button>
            } @empty {
              <p class="text-muted">{{ 'No group matches.' | translate }}</p>
            }
          </div>
        }
        @case ('checked') {
          @if (deviceCount() === 0) {
            <p class="text-warning">
              <i c8yIcon="exclamation-triangle"></i>
              {{ 'This group has no devices directly in it. Pick the group that holds the gateways.' | translate }}
            </p>
          } @else if (missing().length) {
            <p class="text-warning">
              <i c8yIcon="exclamation-triangle"></i>
              {{ missing().length }} {{ 'of' | translate }} {{ deviceCount() }}
              {{ (libraries().length
                ? 'devices in this group lack this file or a point library it references. Add the file to each of them from its OT connectors tab first; that adds its point libraries too:'
                : 'devices in this group do not offer this file, so their operations will fail. Add the file to each of them from its OT connectors tab first:') | translate }}
            </p>
            <ul>
              @for (m of missing(); track m.device.id) {
                <li>{{ m.device['name'] }} <span class="text-muted">· {{ lacks(m) }}</span></li>
              }
            </ul>
          } @else if (deviceCount() === 1) {
            <p>{{ 'The one device in this group offers this file.' | translate }}</p>
          } @else {
            <p>{{ 'All' | translate }} {{ deviceCount() }} {{ 'devices in this group offer this file.' | translate }}</p>
          }
          @if (deviceCount() > 0 && libraries().length) {
            <p class="text-muted">
              {{ 'A group send does not send the point libraries the file references. If you changed one, send it to the group first:' | translate }}
              {{ libraryNames() }}
            </p>
          }
        }
        @case ('running') {
          <p><i c8yIcon="spinner" class="icon-spin"></i> {{ 'Creating the bulk operation' | translate }}…</p>
        }
        @case ('done') {
          <p class="text-success"><i c8yIcon="check-circle"></i> {{ doneText() }}</p>
        }
        @case ('error') {
          <p class="text-danger"><i c8yIcon="exclamation-circle"></i> {{ errorText() }}</p>
        }
      }
    </div>
    <div class="modal-footer">
      @switch (state()) {
        @case ('pick') {
          <button class="btn btn-default" type="button" (click)="close()">{{ 'Cancel' | translate }}</button>
          <button class="btn btn-primary" type="button" [disabled]="!picked()" (click)="check()">{{ 'Next' | translate }}</button>
        }
        @case ('checked') {
          <button class="btn btn-default" type="button" (click)="state.set('pick')">{{ 'Back' | translate }}</button>
          @if (deviceCount() > 0) {
            <button class="btn btn-primary" type="button" (click)="send()">{{ 'Send to the group' | translate }}</button>
          }
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

  readonly state = signal<State>('pick');
  readonly term = signal('');
  readonly groups = signal<IManagedObject[]>([]);
  readonly picked = signal<IManagedObject | undefined>(undefined);
  readonly missing = signal<MissingTypes[]>([]);
  readonly libraries = signal<IManagedObject[]>([]);
  readonly deviceCount = signal(0);
  readonly doneText = signal('');
  readonly errorText = signal('');

  get file(): ManagedFile | undefined {
    return fileOfType(this.entry?.['configurationType']);
  }

  get isLibrary(): boolean {
    return this.file?.kind === 'library';
  }

  libraryNames(): string {
    return this.libraries()
      .map(l => l['name'])
      .join(', ');
  }

  /** What a device lacks, for people. */
  lacks(missing: MissingTypes): string {
    return missing.types
      .map(type => {
        if (type === this.file?.type) {
          return 'this file';
        }
        const library = this.libraries().find(l => l['configurationType'] === type);
        return `point library ${library?.['name'] ?? fileOfType(type)?.name ?? type}`;
      })
      .join(', ');
  }

  constructor() {
    void this.search();
  }

  async search(term = this.term()): Promise<void> {
    this.groups.set(await this.ops.findGroups(term));
  }

  async check(): Promise<void> {
    const group = this.picked();
    if (!group) {
      return;
    }
    const file = this.file;
    if (!this.entry || !file) {
      return;
    }
    try {
      const libraries = file.kind === 'connector' ? await this.ops.librariesOf(this.entry) : [];
      this.libraries.set(libraries);
      const types = [file.type, ...libraries.map(l => l['configurationType'] as string)];
      const { devices, missing } = await this.ops.checkGroup(group.id, types);
      this.deviceCount.set(devices);
      this.missing.set(missing);
      this.state.set('checked');
    } catch (e) {
      this.errorText.set(e instanceof Error ? e.message : String(e));
      this.state.set('error');
    }
  }

  async send(): Promise<void> {
    const group = this.picked();
    if (!group || !this.entry) {
      return;
    }
    this.state.set('running');
    try {
      const bulk = await this.ops.sendToGroup(group.id, this.entry);
      this.doneText.set(
        `Bulk operation ${bulk.id} sends ${this.entry['name']} to ${group['name']}. Its progress is under Device control > Bulk operations.`
      );
      this.state.set('done');
    } catch (e) {
      this.errorText.set(e instanceof Error ? e.message : String(e));
      this.state.set('error');
    }
  }

  close(): void {
    this.modalRef.hide();
  }
}
