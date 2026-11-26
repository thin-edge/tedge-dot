import { NgTemplateOutlet } from '@angular/common';
import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { IManagedObject, InventoryService } from '@c8y/client';
import { CoreModule, Permissions } from '@c8y/ngx-components';
import { gettext } from '@c8y/ngx-components/gettext';
import { SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import { BsModalRef } from 'ngx-bootstrap/modal';

import { PROTOCOL_IDS } from '../generated/config-builder';
import { ConnectorOperationsService, RegistrationCancelled } from '../shared/connector-operations.service';
import { newFileRoute } from '../shared/editor-context';
import {
  type FileKind,
  type ManagedFile,
  entriesOfKind,
  fileOfType,
  isValidConnectorName,
  isValidName,
  libraryPathOf,
  managedFile,
  normalizeName,
  pathOf,
} from '../shared/naming';
import { canAddEntries } from '../shared/permissions';
import { managedEntriesQuery } from '../shared/queries';
import { ValidIfDirective } from './valid-if.directive';

type State = 'pick' | 'running' | 'confirm' | 'done' | 'error';

/** What "Add connector file" on the device tab does. */
type Choice = 'repository' | 'new' | 'register';

const CHOICES: { id: Choice; label: string; text: string }[] = [
  {
    id: 'repository',
    label: gettext('From the repository'),
    text: gettext('Send a connector file or point library from the configuration repository to this gateway.'),
  },
  {
    id: 'new',
    label: gettext('New file'),
    text: gettext('Create a connector file or point library in the repository, then send it to this gateway.'),
  },
  {
    id: 'register',
    label: gettext('Already on the gateway'),
    text: gettext('Register a file the gateway already has, such as a packaged modbus.toml. Nothing is created or sent.'),
  },
];

/**
 * Adds a connector file or point library to one gateway: registers it in thin-edge.io's list of
 * managed files when the gateway does not offer its type yet, then sends the repository entry. A
 * connector file brings the repository libraries it references: those the gateway does not offer
 * are registered in the same edit and sent first.
 *
 * Opened from the OT connectors page (with an entry, to pick a gateway), from the editor (with an
 * entry and a gateway, after saving), or from the device tab (with a gateway). From the device
 * tab it starts with a choice: send an entry from the repository, create a new file in the editor,
 * or register a file that is already on the gateway.
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
          } @else if (entry) {
            <p>{{ 'Gateway' | translate }}: <strong>{{ device['name'] }}</strong></p>
          }
          @if (entry) {
            <p>{{ kindLabel(entry) | translate }}: <strong>{{ entry['name'] }}</strong> · {{ fileLabel(entry) }}</p>
          } @else {
            <fieldset class="m-b-16">
              <legend class="sr-only">{{ 'What to add' | translate }}</legend>
              @for (c of choices; track c.id) {
                <label class="c8y-radio m-b-8">
                  <input type="radio" name="tdot-choice" [value]="c.id" [checked]="choice() === c.id" (change)="choose(c.id)" />
                  <span></span>
                  <span>
                    <strong>{{ c.label | translate }}</strong>
                    <span class="d-block text-muted small">{{ c.text | translate }}</span>
                  </span>
                </label>
              }
            </fieldset>
            @switch (choice()) {
              @case ('repository') {
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
              }
              @case ('new') {
                <ng-container *ngTemplateOutlet="kindPicker"></ng-container>
                <p class="text-muted">
                  {{ 'Opens the editor. When you save, the file is added to the repository and you can send it to this gateway.' | translate }}
                </p>
              }
              @case ('register') {
                <ng-container *ngTemplateOutlet="kindPicker"></ng-container>
                <div class="row">
                  @if (kind() === 'library') {
                    <div class="col-sm-4">
                      <c8y-form-group>
                        <label for="tdot-reg-protocol">{{ 'Protocol' | translate }}</label>
                        <div class="c8y-select-wrapper">
                          <select id="tdot-reg-protocol" class="form-control" [(ngModel)]="protocol">
                            <option value="" disabled>{{ 'Select a protocol' | translate }}</option>
                            @for (p of protocols; track p) {
                              <option [value]="p">{{ p }}</option>
                            }
                          </select>
                        </div>
                      </c8y-form-group>
                    </div>
                  }
                  <div [class]="kind() === 'library' ? 'col-sm-8' : 'col-sm-12'">
                    <c8y-form-group>
                      <label for="tdot-reg-name">{{ (kind() === 'library' ? 'Library name' : 'File name') | translate }}</label>
                      <div class="input-group">
                        <input
                          id="tdot-reg-name"
                          class="form-control"
                          aria-describedby="tdot-reg-name-msg"
                          [(ngModel)]="fileName"
                          [tdotValidIf]="registerNameCheck()"
                          (blur)="fileTouched.set(true)"
                          [placeholder]="kind() === 'library' ? 'opcua-demo-pump' : 'modbus'"
                        />
                        <span class="input-group-addon">.toml</span>
                      </div>
                      <c8y-messages id="tdot-reg-name-msg" [defaults]="noDefaultMessages" [helpMessage]="registerNameError() ? '' : registerPath()">
                        @if (registerNameError()) {
                          <c8y-message>{{ registerNameRule() | translate }}</c8y-message>
                        }
                      </c8y-messages>
                    </c8y-form-group>
                  </div>
                </div>
                <p class="text-muted">
                  {{ 'Registering makes a file that is already on the gateway a configuration type, so you can take snapshots of it and send new versions. It does not create or send the file.' | translate }}
                  @if (canAdd) {
                    {{ 'For a new file, choose New file.' | translate }}
                  }
                </p>
              }
            }
            <ng-template #kindPicker>
              <div class="form-group">
                <label class="c8y-radio radio-inline">
                  <input type="radio" name="tdot-kind" value="connector" [checked]="kind() === 'connector'" (change)="kind.set('connector')" />
                  <span></span>
                  <span>{{ 'Connector file' | translate }}</span>
                </label>
                <label class="c8y-radio radio-inline">
                  <input type="radio" name="tdot-kind" value="library" [checked]="kind() === 'library'" (change)="kind.set('library')" />
                  <span></span>
                  <span>{{ 'Point library' | translate }}</span>
                </label>
              </div>
            </ng-template>
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
            {{ primaryLabel | translate }}
          </button>
        }
        @case ('confirm') {
          <button class="btn btn-default" type="button" (click)="answer(false)">{{ 'Cancel' | translate }}</button>
          <button class="btn btn-primary" type="button" (click)="answer(true)">
            {{ (registering ? 'Register' : 'Register and send') | translate }}
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
  private readonly router = inject(Router);
  private readonly permissions = inject(Permissions);

  /** Set by the opener. */
  entry?: IManagedObject;
  device?: IManagedObject;
  onFinished?: () => void;

  readonly protocols = PROTOCOL_IDS;
  /** Whether the user can add repository entries; without it, "New file" is not offered. */
  readonly canAdd = canAddEntries(roles => this.permissions.hasAnyRole(roles));
  readonly choices = CHOICES.filter(c => c.id !== 'new' || this.canAdd);

  readonly state = signal<State>('pick');
  readonly choice = signal<Choice>('repository');
  readonly kind = signal<FileKind>('connector');
  readonly term = signal('');
  readonly gateways = signal<IManagedObject[]>([]);
  readonly connectorEntries = signal<IManagedObject[]>([]);
  readonly libraryEntries = signal<IManagedObject[]>([]);
  readonly librariesToSend = signal<string[]>([]);
  readonly picked = signal<IManagedObject | undefined>(undefined);
  readonly chosen = signal<IManagedObject | undefined>(undefined);
  readonly fileName = signal('');
  readonly protocol = signal('');
  readonly fileTouched = signal(false);
  readonly step = signal('');
  readonly block = signal('');
  readonly doneText = signal('');
  readonly errorText = signal('');
  private resolveConfirm?: (ok: boolean) => void;

  /** The file "Already on the gateway" registers, once its name (and protocol) are valid. */
  readonly registerFile = computed(() => managedFile(this.kind(), this.fileName(), this.protocol()));
  /** An empty name is not an error yet; the Register button waits for a valid one. */
  readonly registerNameCheck = computed(() =>
    this.kind() === 'library'
      ? (value: string) => !value || isValidName(normalizeName(value))
      : (value: string) => !value || isValidConnectorName(normalizeName(value))
  );
  readonly registerNameError = computed(() => this.fileTouched() && !this.registerNameCheck()(this.fileName()));
  readonly registerNameRule = computed(() =>
    this.kind() === 'library'
      ? gettext('Letters, digits, ".", "_" and "-" only, starting with a letter or digit.')
      : gettext('Letters, digits, ".", "_" and "-" only, starting with a letter or digit. It may not start with "points.", which point libraries use.')
  );
  /** Where the registered file is on the gateway, with placeholders until it is valid. */
  readonly registerPath = computed(() => {
    const file = this.registerFile();
    if (file) {
      return file.path;
    }
    return this.kind() === 'library' ? libraryPathOf(this.protocol() || '<protocol>', '<name>') : pathOf('<name>');
  });
  /** No built-in texts, such as "This field is required.": the field shows its own. */
  readonly noDefaultMessages = {};

  /** Whether this run only registers a file, from "Already on the gateway". */
  get registering(): boolean {
    return !this.entry && this.choice() === 'register';
  }

  get title(): string {
    if (!this.entry && this.device) {
      return 'Add connector file';
    }
    return this.entry && fileOfType(this.entry['configurationType'])?.kind === 'library'
      ? 'Add point library to a gateway'
      : 'Add connector file to a gateway';
  }

  get primaryLabel(): string {
    if (this.entry) {
      return 'Send';
    }
    return { repository: 'Send', new: 'Open the editor', register: 'Register' }[this.choice()];
  }

  get entryType(): string | undefined {
    return (this.entry ?? this.chosen())?.['configurationType'];
  }

  get ready(): boolean {
    const gateway = this.device ?? this.picked();
    if (!gateway) {
      return false;
    }
    if (this.entry) {
      return true;
    }
    switch (this.choice()) {
      case 'repository':
        return !!this.chosen();
      case 'new':
        return this.canAdd;
      case 'register':
        return !!this.registerFile();
    }
  }

  async ngOnInit(): Promise<void> {
    if (!this.device) {
      await this.search();
    }
    if (!this.entry) {
      const { data } = await this.inventory.list({ query: managedEntriesQuery(), pageSize: 2000 });
      this.connectorEntries.set(entriesOfKind(data, 'connector'));
      this.libraryEntries.set(entriesOfKind(data, 'library'));
    }
  }

  choose(choice: Choice): void {
    this.choice.set(choice);
    this.fileTouched.set(false);
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
    if (!gateway) {
      return;
    }
    if (!this.entry && this.choice() === 'new') {
      this.openEditor(gateway);
      return;
    }
    const entry = this.registering ? undefined : (this.entry ?? this.chosen());
    const file: ManagedFile | undefined = this.registering ? this.registerFile() : fileOfType(entry?.['configurationType']);
    if (!file) {
      return;
    }
    this.state.set('running');
    this.librariesToSend.set([]);
    try {
      let libraries: IManagedObject[] = [];
      if (entry && file.kind === 'connector') {
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
      if (!entry) {
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

  /** "New file": the editor, which offers to send the saved file to this gateway. */
  private openEditor(gateway: IManagedObject): void {
    const route = newFileRoute(this.kind(), String(gateway.id));
    this.modalRef.hide();
    void this.router.navigate(route.commands, { queryParams: route.queryParams });
  }

  close(): void {
    this.resolveConfirm?.(false);
    this.modalRef.hide();
  }
}
