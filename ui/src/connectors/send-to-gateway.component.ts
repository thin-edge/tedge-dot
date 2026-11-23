import { NgTemplateOutlet } from '@angular/common';
import { Component, OnInit, inject, signal } from '@angular/core';
import { IManagedObject, InventoryService } from '@c8y/client';
import { CoreModule } from '@c8y/ngx-components';
import { SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { ConnectorOperationsService, RegistrationCancelled } from '../shared/connector-operations.service';
import {
  type ManagedFile,
  connectorFile,
  entriesOfKind,
  fileOfType,
  isValidConnectorName,
  normalizeName,
} from '../shared/naming';
import { managedEntriesQuery } from '../shared/queries';
import { ValidIfDirective } from './valid-if.directive';

type State = 'pick' | 'running' | 'confirm' | 'done' | 'error';

/**
 * Adds a connector file or point library to one gateway: registers it in thin-edge.io's list of
 * managed files when the gateway does not offer its type yet, then sends the repository entry. A
 * connector file brings the repository libraries it references: those the gateway does not offer
 * are registered in the same edit and sent first.
 *
 * Opened from the OT connectors page (with an entry, to pick a gateway), from the device tab
 * (with a gateway, to pick an entry), or from the device tab to register a file that is already
 * on the gateway (`registerOnly`).
 */
@Component({
  selector: 'tdot-send-to-gateway',
  imports: [CoreModule, SharedRepositoryModule, NgTemplateOutlet, ValidIfDirective],
  providers: [ConnectorOperationsService],
  template: `
    <div class="modal-header dialog-header">
      <i c8yIcon="plug"></i>
      <h4>{{ title | translate }}</h4>
    </div>
    <div class="modal-body">
      @switch (state()) {
        @case ('pick') {
          @if (!device) {
            <c8y-form-group>
              <label for="tdot-gw-search">{{ 'Gateway' | translate }}</label>
              <input id="tdot-gw-search" class="form-control" [(ngModel)]="term" (ngModelChange)="search($event)"
                [placeholder]="'Search thin-edge.io gateways by name' | translate" />
            </c8y-form-group>
            <div class="list-group">
              @for (g of gateways(); track g.id) {
                <button type="button" class="list-group-item" [class.active]="picked()?.id === g.id" (click)="picked.set(g)">
                  {{ g['name'] }}
                  @if (ops.offers(g, entryType)) {
                    <span class="tag tag--info m-l-8">{{ 'has this file' | translate }}</span>
                  }
                </button>
              } @empty {
                <p class="text-muted">{{ 'No thin-edge.io gateway matches.' | translate }}</p>
              }
            </div>
          }
          @if (registerOnly) {
            <c8y-form-group>
              <label for="tdot-reg-name">{{ 'File in /etc/tedge/plugins/ot' | translate }}</label>
              <input
                id="tdot-reg-name"
                class="form-control"
                aria-describedby="tdot-reg-name-msg"
                [(ngModel)]="fileName"
                [tdotValidIf]="registerNameCheck"
                (blur)="fileTouched.set(true)"
                placeholder="modbus"
              />
              <c8y-messages id="tdot-reg-name-msg">
                @if (fileTouched() && !registerNameCheck(fileName())) {
                  <c8y-message>
                    {{ 'Letters, digits, ".", "_" and "-" only, starting with a letter or digit. It may not start with "points.", which point libraries use.' | translate }}
                  </c8y-message>
                }
              </c8y-messages>
            </c8y-form-group>
            <p class="text-muted">
              {{ 'The gateway then offers the file as a configuration type, so you can take snapshots of it and send new versions.' | translate }}
            </p>
          } @else if (!entry) {
            <label>{{ 'Connector file' | translate }}</label>
            <div class="list-group">
              @for (e of connectorEntries(); track e.id) {
                <ng-container *ngTemplateOutlet="entryItem; context: { $implicit: e }"></ng-container>
              } @empty {
                <p class="text-muted">{{ 'The repository has no connector files yet.' | translate }}</p>
              }
            </div>
            @if (libraryEntries().length) {
              <label>{{ 'Point library' | translate }}</label>
              <div class="list-group">
                @for (e of libraryEntries(); track e.id) {
                  <ng-container *ngTemplateOutlet="entryItem; context: { $implicit: e }"></ng-container>
                }
              </div>
            }
            <ng-template #entryItem let-e>
              <button type="button" class="list-group-item" [class.active]="chosen()?.id === e.id" (click)="chosen.set(e)">
                <strong>{{ e['name'] }}</strong> · {{ fileLabel(e) }}
                @if (device && ops.offers(device, e['configurationType'])) {
                  <span class="tag tag--info m-l-8">{{ 'on this gateway' | translate }}</span>
                }
                @if (e['description']) {
                  <div class="text-muted small">{{ e['description'] }}</div>
                }
              </button>
            </ng-template>
          } @else {
            <p>{{ kindLabel(entry) | translate }}: <strong>{{ entry['name'] }}</strong> · {{ fileLabel(entry) }}</p>
          }
        }
        @case ('running') {
          <p><i c8yIcon="spinner" class="icon-spin"></i> {{ step() }}…</p>
        }
        @case ('confirm') {
          <p>
            {{ 'The gateway does not offer every file yet. These entries will be appended to its /etc/tedge/plugins/tedge-configuration-plugin.toml; nothing else in that file changes:' | translate }}
          </p>
          <pre class="m-b-0">{{ block() }}</pre>
          @if (librariesToSend().length) {
            <p class="m-t-16 m-b-0">
              {{ 'The connector file references point libraries the gateway does not have yet. They are sent first:' | translate }}
              {{ librariesToSend().join(', ') }}
            </p>
          }
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
          <button class="btn btn-primary" type="button" [disabled]="!ready" (click)="run()">
            {{ (registerOnly ? 'Register' : 'Send') | translate }}
          </button>
        }
        @case ('confirm') {
          <button class="btn btn-default" type="button" (click)="answer(false)">{{ 'Cancel' | translate }}</button>
          <button class="btn btn-primary" type="button" (click)="answer(true)">
            {{ (registerOnly ? 'Register' : 'Register and send') | translate }}
          </button>
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
export class SendToGatewayComponent implements OnInit {
  readonly ops = inject(ConnectorOperationsService);
  private readonly inventory = inject(InventoryService);
  private readonly modalRef = inject(BsModalRef);

  /** Set by the opener. */
  entry?: IManagedObject;
  device?: IManagedObject;
  registerOnly = false;
  onFinished?: () => void;

  readonly state = signal<State>('pick');
  readonly term = signal('');
  readonly gateways = signal<IManagedObject[]>([]);
  readonly connectorEntries = signal<IManagedObject[]>([]);
  readonly libraryEntries = signal<IManagedObject[]>([]);
  readonly librariesToSend = signal<string[]>([]);
  readonly picked = signal<IManagedObject | undefined>(undefined);
  readonly chosen = signal<IManagedObject | undefined>(undefined);
  readonly fileName = signal('');
  readonly fileTouched = signal(false);
  readonly step = signal('');
  readonly block = signal('');
  readonly doneText = signal('');
  readonly errorText = signal('');
  private resolveConfirm?: (ok: boolean) => void;

  get title(): string {
    if (this.registerOnly) {
      return 'Register a file on the gateway';
    }
    return this.entry && fileOfType(this.entry['configurationType'])?.kind === 'library'
      ? 'Add point library to a gateway'
      : 'Add connector file to a gateway';
  }

  get entryType(): string | undefined {
    return (this.entry ?? this.chosen())?.['configurationType'];
  }

  /** An empty name is not an error yet; the Register button waits for a valid one. */
  readonly registerNameCheck = (value: string) => !value || isValidConnectorName(normalizeName(value));

  get fileValid(): boolean {
    return isValidConnectorName(normalizeName(this.fileName()));
  }

  get ready(): boolean {
    const gateway = this.device ?? this.picked();
    if (!gateway) {
      return false;
    }
    return this.registerOnly ? this.fileValid : !!(this.entry ?? this.chosen());
  }

  async ngOnInit(): Promise<void> {
    if (!this.device) {
      await this.search();
    }
    if (!this.entry && !this.registerOnly) {
      const { data } = await this.inventory.list({ query: managedEntriesQuery(), pageSize: 2000 });
      this.connectorEntries.set(entriesOfKind(data, 'connector'));
      this.libraryEntries.set(entriesOfKind(data, 'library'));
    }
  }

  /** Where the entry's file lands, relative to /etc/tedge/plugins/ot. */
  fileLabel(entry: IManagedObject): string {
    const file = fileOfType(entry['configurationType']);
    if (!file) {
      return '';
    }
    return file.kind === 'library' ? `points.d/${file.protocol}/${file.name}.toml` : `${file.name}.toml`;
  }

  kindLabel(entry: IManagedObject): string {
    return fileOfType(entry['configurationType'])?.kind === 'library' ? 'Point library' : 'Connector file';
  }

  async search(term = this.term()): Promise<void> {
    this.gateways.set(await this.ops.findGateways(term));
  }

  answer(ok: boolean): void {
    this.resolveConfirm?.(ok);
  }

  async run(): Promise<void> {
    const gateway = this.device ?? this.picked();
    const entry = this.entry ?? this.chosen();
    const file: ManagedFile | undefined = this.registerOnly
      ? connectorFile(normalizeName(this.fileName()))
      : fileOfType(entry?.['configurationType']);
    if (!gateway || !file) {
      return;
    }
    this.state.set('running');
    this.librariesToSend.set([]);
    try {
      let libraries: IManagedObject[] = [];
      if (!this.registerOnly && entry && file.kind === 'connector') {
        this.step.set('Looking up the point libraries the connector file references');
        libraries = await this.ops.librariesOf(entry);
      }
      const toSend = libraries.filter(l => !this.ops.offers(gateway, l['configurationType']));
      this.librariesToSend.set(toSend.map(l => l['name']));
      const files = [file, ...libraries.map(l => fileOfType(l['configurationType'])!)];
      const updated = await this.ops.register(
        gateway,
        files,
        block =>
          new Promise<boolean>(resolve => {
            this.block.set(block);
            this.state.set('confirm');
            this.resolveConfirm = ok => {
              this.state.set('running');
              resolve(ok);
            };
          }),
        step => this.step.set(step)
      );
      if (this.registerOnly || !entry) {
        this.doneText.set(`${gateway['name']} offers ${file.type} for ${file.path}.`);
      } else {
        for (const library of toSend) {
          this.step.set(`Sending the point library ${library['name']}`);
          await this.ops.sendToGatewayAndWait(updated, library);
        }
        this.step.set(file.kind === 'library' ? 'Sending the point library' : 'Sending the connector file');
        const operation = await this.ops.sendToGateway(updated, entry);
        const after = toSend.length ? `, after the point ${toSend.length === 1 ? 'library' : 'libraries'} ${toSend.map(l => l['name']).join(', ')}` : '';
        this.doneText.set(
          `Operation ${operation.id} sends ${entry['name']} to ${gateway['name']}${after}. Its status is on the gateway's Control tab.`
        );
      }
      this.state.set('done');
      this.onFinished?.();
    } catch (e) {
      if (e instanceof RegistrationCancelled) {
        this.state.set('pick');
        return;
      }
      this.errorText.set(e instanceof Error ? e.message : String(e));
      this.state.set('error');
    }
  }

  close(): void {
    this.resolveConfirm?.(false);
    this.modalRef.hide();
  }
}
