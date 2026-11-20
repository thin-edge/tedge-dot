import { Component, OnInit, inject, signal } from '@angular/core';
import { IManagedObject, InventoryService } from '@c8y/client';
import { CoreModule } from '@c8y/ngx-components';
import { SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { ConnectorOperationsService, RegistrationCancelled } from '../shared/connector-operations.service';
import { isValidName, nameOf, normalizeName, pathOf, typeOf } from '../shared/naming';
import { connectorEntriesQuery } from '../shared/queries';

type State = 'pick' | 'running' | 'confirm' | 'done' | 'error';

/**
 * Adds a connector file to one gateway: registers it in thin-edge.io's list of managed files when
 * the gateway does not offer its type yet, then sends the repository entry.
 *
 * Opened from the OT connectors page (with an entry, to pick a gateway), from the device tab
 * (with a gateway, to pick an entry), or from the device tab to register a file that is already
 * on the gateway (`registerOnly`).
 */
@Component({
  selector: 'tdot-send-to-gateway',
  imports: [CoreModule, SharedRepositoryModule],
  providers: [ConnectorOperationsService],
  template: `
    <div class="modal-header dialog-header">
      <i c8yIcon="plug"></i>
      <h4>{{ (registerOnly ? 'Register a file on the gateway' : 'Add connector file to a gateway') | translate }}</h4>
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
                  @if (entryName && ops.offers(g, entryName)) {
                    <span class="tag tag--info m-l-8">{{ 'has this file' | translate }}</span>
                  }
                </button>
              } @empty {
                <p class="text-muted">{{ 'No thin-edge.io gateway matches.' | translate }}</p>
              }
            </div>
          }
          @if (registerOnly) {
            <c8y-form-group [hasError]="!!fileName() && !fileValid">
              <label for="tdot-reg-name">{{ 'File in /etc/tedge/plugins/ot' | translate }}</label>
              <input id="tdot-reg-name" class="form-control" [(ngModel)]="fileName" placeholder="modbus" />
            </c8y-form-group>
            <p class="text-muted">
              {{ 'The gateway then offers the file as a configuration type, so you can take snapshots of it and send new versions.' | translate }}
            </p>
          } @else if (!entry) {
            <label>{{ 'Connector file' | translate }}</label>
            <div class="list-group">
              @for (e of entries(); track e.id) {
                <button type="button" class="list-group-item" [class.active]="chosen()?.id === e.id" (click)="chosen.set(e)">
                  <strong>{{ e['name'] }}</strong> · {{ fileOf(e) }}
                  @if (device && ops.offers(device, nameOf(e['configurationType']) ?? '')) {
                    <span class="tag tag--info m-l-8">{{ 'on this gateway' | translate }}</span>
                  }
                  @if (e['description']) {
                    <div class="text-muted small">{{ e['description'] }}</div>
                  }
                </button>
              } @empty {
                <p class="text-muted">{{ 'The repository has no connector files yet.' | translate }}</p>
              }
            </div>
          } @else {
            <p>{{ 'Connector file' | translate }}: <strong>{{ entry['name'] }}</strong> · {{ fileOf(entry) }}</p>
          }
        }
        @case ('running') {
          <p><i c8yIcon="spinner" class="icon-spin"></i> {{ step() }}…</p>
        }
        @case ('confirm') {
          <p>
            {{ 'The gateway does not offer this file yet. This entry will be appended to its /etc/tedge/plugins/tedge-configuration-plugin.toml; nothing else in that file changes:' | translate }}
          </p>
          <pre class="m-b-0">{{ block() }}</pre>
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

  readonly nameOf = nameOf;
  readonly state = signal<State>('pick');
  readonly term = signal('');
  readonly gateways = signal<IManagedObject[]>([]);
  readonly entries = signal<IManagedObject[]>([]);
  readonly picked = signal<IManagedObject | undefined>(undefined);
  readonly chosen = signal<IManagedObject | undefined>(undefined);
  readonly fileName = signal('');
  readonly step = signal('');
  readonly block = signal('');
  readonly doneText = signal('');
  readonly errorText = signal('');
  private resolveConfirm?: (ok: boolean) => void;

  get entryName(): string | undefined {
    return nameOf((this.entry ?? this.chosen())?.['configurationType']);
  }

  get fileValid(): boolean {
    return isValidName(normalizeName(this.fileName()));
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
      const { data } = await this.inventory.list({ query: connectorEntriesQuery(), pageSize: 2000 });
      this.entries.set(data);
    }
  }

  fileOf(entry: IManagedObject): string {
    const name = nameOf(entry['configurationType']);
    return name ? `${name}.toml` : '';
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
    const name = this.registerOnly ? normalizeName(this.fileName()) : nameOf(entry?.['configurationType']);
    if (!gateway || !name) {
      return;
    }
    this.state.set('running');
    try {
      const updated = await this.ops.register(
        gateway,
        name,
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
        this.doneText.set(`${gateway['name']} offers ${typeOf(name)} for ${pathOf(name)}.`);
      } else {
        this.step.set('Sending the connector file');
        const operation = await this.ops.sendToGateway(updated, entry);
        this.doneText.set(`Operation ${operation.id} sends ${entry['name']} to ${gateway['name']}. Its status is on the gateway's Control tab.`);
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
