import { Component, computed, effect, inject, signal } from '@angular/core';
import { takeUntilDestroyed } from '@angular/core/rxjs-interop';
import { ActivatedRoute, Router } from '@angular/router';
import { IManagedObject, InventoryService } from '@c8y/client';
import { AlertService, CoreModule, ModalService, Status } from '@c8y/ngx-components';
import { gettext } from '@c8y/ngx-components/gettext';
import { EditorComponent, loadMonacoEditor } from '@c8y/ngx-components/editor';
import { RepositoryService, RepositoryType, SharedRepositoryModule } from '@c8y/ngx-components/repository/shared';
import type * as Monaco from 'monaco-editor';

import { PROTOCOL_IDS, emitToml, newConfig, type Config, type ProtocolId } from '../generated/config-builder';
import { commentLines } from '../shared/form-model';
import { writeKeepingLayout } from '../shared/toml-write';
import { isValidName, nameOf, normalizeName, pathOf, typeOf } from '../shared/naming';
import { TOML_LANGUAGE_ID, tomlLanguage } from '../shared/toml-language';
import { type ValidationResult, isValid, validateConnectorFile } from '../shared/validation';
import { ConnectorFormComponent } from './connector-form.component';

const MARKER_OWNER = 'tedge-dot';

/** Adds or edits one connector file in the configuration repository. */
@Component({
  selector: 'tdot-connector-editor',
  imports: [CoreModule, SharedRepositoryModule, EditorComponent, ConnectorFormComponent],
  template: `
    <c8y-title>{{ (isNew() ? 'Add connector file' : 'Edit connector file') | translate }}</c8y-title>

    <div class="card content-fullpage d-flex d-col">
      <div class="card-header separator">
        <div class="card-title">{{ isNew() ? ('New connector file' | translate) : entry()?.['name'] }}</div>
      </div>
      <div class="card-block d-flex d-col flex-grow min-height-0">
        <div class="row">
          <div class="col-md-3">
            <c8y-form-group [hasError]="nameTouched() && !name().trim()">
              <label for="tdot-name">{{ 'Name' | translate }}</label>
              <input id="tdot-name" class="form-control" [(ngModel)]="name" (blur)="nameTouched.set(true)" />
            </c8y-form-group>
          </div>
          <div class="col-md-3">
            <c8y-form-group [hasError]="fileTouched() && !fileValid()">
              <label for="tdot-file">{{ 'File name' | translate }}</label>
              <div class="input-group">
                <input
                  id="tdot-file"
                  class="form-control"
                  [(ngModel)]="fileName"
                  [disabled]="!isNew()"
                  (blur)="fileTouched.set(true)"
                  placeholder="opcua-pump"
                />
                <span class="input-group-addon">.toml</span>
              </div>
              @if (fileTouched() && !fileValid()) {
                <c8y-messages>
                  <c8y-message>{{ 'Letters, digits, ".", "_" and "-" only, starting with a letter or digit.' | translate }}</c8y-message>
                </c8y-messages>
              } @else {
                <p class="help-block text-muted small text-truncate" [title]="gatewayPath()">{{ gatewayPath() }}</p>
              }
            </c8y-form-group>
          </div>
          <div class="col-md-3">
            <c8y-form-group>
              <label for="tdot-device-type">{{ 'Device type' | translate }}</label>
              <input id="tdot-device-type" class="form-control" [(ngModel)]="deviceType" [placeholder]="'Any' | translate" />
            </c8y-form-group>
          </div>
          @if (isNew()) {
            <div class="col-md-3">
              <c8y-form-group>
                <label for="tdot-start">{{ 'Start from' | translate }}</label>
                <div class="c8y-select-wrapper">
                  <select id="tdot-start" class="form-control" [(ngModel)]="startFrom" (ngModelChange)="useTemplate($event)">
                    @for (p of protocols; track p) {
                      <option [ngValue]="p">{{ p }}</option>
                    }
                  </select>
                </div>
              </c8y-form-group>
            </div>
          }
          <div class="col-md-12">
            <c8y-form-group>
              <label for="tdot-description">{{ 'Description' | translate }}</label>
              <input id="tdot-description" class="form-control" [(ngModel)]="description" />
            </c8y-form-group>
          </div>
        </div>

        <div class="m-b-8">
          <div class="btn-group btn-group-sm" role="group" [attr.aria-label]="'View' | translate">
            <button type="button" class="btn btn-default" [class.active]="view() === 'form'" [attr.aria-pressed]="view() === 'form'" (click)="showForm()">
              {{ 'Form' | translate }}
            </button>
            <button type="button" class="btn btn-default" [class.active]="view() === 'toml'" [attr.aria-pressed]="view() === 'toml'" (click)="showToml()">
              TOML
            </button>
          </div>
        </div>

        @if (view() === 'form') {
          <div class="tdot-pane flex-grow">
            @if (result(); as r) {
              @if (r.syntax) {
                <div class="alert alert-danger" role="alert">
                  {{ 'The TOML has a syntax error on line' | translate }} {{ r.syntax.line }}{{ ', so the form cannot show it. Fix it in the TOML view.' | translate }}
                  <button type="button" class="btn btn-default btn-sm m-l-8" (click)="showToml()">{{ 'Open the TOML view' | translate }}</button>
                </div>
              } @else if (!r.config?.['connector']) {
                <div class="alert alert-warning" role="alert">
                  {{ 'This file has no [connector] table, so the form cannot show it. Edit it in the TOML view.' | translate }}
                </div>
              } @else {
                <tdot-connector-form [config]="r.config!" [errors]="r.fields" (configChange)="onFormChange($event)" />
              }
            }
          </div>
        } @else {
          <div class="tdot-editor flex-grow">
            <c8y-editor
              [(ngModel)]="content"
              [editorOptions]="editorOptions"
              (editorInit)="onEditorInit($event)"
            ></c8y-editor>
          </div>
        }

        <div class="p-t-8">
          @if (result(); as r) {
            @if (r.syntax) {
              <p class="text-danger">
                <i c8yIcon="exclamation-circle"></i>
                {{ 'Line' | translate }} {{ r.syntax.line }}, {{ 'column' | translate }} {{ r.syntax.column }}:
                {{ r.syntax.message }}
              </p>
            } @else if (r.fields.length) {
              <ul class="list-unstyled text-danger m-b-0">
                @for (f of r.fields; track f.path + f.message) {
                  <li><i c8yIcon="exclamation-circle"></i> {{ f.label }}: {{ f.message }}</li>
                }
              </ul>
            } @else {
              <p class="text-success m-b-0">
                <i c8yIcon="check-circle"></i>
                {{ 'Matches the contract schema. The gateway checks point libraries and files when it loads the file.' | translate }}
              </p>
            }
          }
        </div>
      </div>
      <div class="card-footer separator text-right">
        <button class="btn btn-default" type="button" (click)="cancel()">{{ 'Cancel' | translate }}</button>
        <button class="btn btn-primary" type="button" [disabled]="!canSave() || saving()" (click)="save()">
          {{ 'Save' | translate }}
        </button>
      </div>
    </div>
  `,
  styles: [
    `
      .tdot-editor {
        min-height: 360px;
        height: 55vh;
        border: 1px solid var(--c8y-form-control-border-color-default, #ccc);
      }
      .tdot-editor c8y-editor {
        display: block;
        height: 100%;
      }
      .tdot-pane {
        min-height: 360px;
        height: 55vh;
        overflow: auto;
        padding: 4px 8px 4px 0;
      }
    `,
  ],
})
export class ConnectorEditorComponent {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly inventory = inject(InventoryService);
  private readonly repository = inject(RepositoryService);
  private readonly alert = inject(AlertService);
  private readonly modal = inject(ModalService);

  readonly protocols = PROTOCOL_IDS;
  readonly editorOptions: Monaco.editor.IStandaloneEditorConstructionOptions = {
    language: TOML_LANGUAGE_ID,
    minimap: { enabled: false },
    automaticLayout: true,
    scrollBeyondLastLine: false,
    tabSize: 2,
  };

  readonly isNew = signal(true);
  readonly entry = signal<IManagedObject | undefined>(undefined);
  readonly name = signal('');
  readonly description = signal('');
  readonly fileName = signal('');
  readonly deviceType = signal('');
  readonly startFrom = signal<ProtocolId>('opcua');
  readonly content = signal('');
  readonly nameTouched = signal(false);
  readonly fileTouched = signal(false);
  readonly saving = signal(false);
  readonly view = signal<'form' | 'toml'>('form');

  readonly result = computed<ValidationResult>(() => validateConnectorFile(this.content() ?? ''));
  readonly fileValid = computed(() => isValidName(normalizeName(this.fileName())));
  /** Where the file lands on a gateway, shown under the file name. */
  readonly gatewayPath = computed(() => pathOf(this.fileValid() ? normalizeName(this.fileName()) : '<name>'));
  readonly canSave = computed(() => !!this.name().trim() && this.fileValid() && isValid(this.result()));

  private monaco?: typeof Monaco;
  private editor?: Monaco.editor.IStandaloneCodeEditor;

  constructor() {
    effect(() => this.applyMarkers(this.result()));
    // The router keeps this component when only the id changes, e.g. from an entry to "new".
    this.route.paramMap.pipe(takeUntilDestroyed()).subscribe(params => void this.load(params.get('id')));
  }

  private async load(id: string | null): Promise<void> {
    this.nameTouched.set(false);
    this.fileTouched.set(false);
    if (!id || id === 'new') {
      this.showForm();
      this.isNew.set(true);
      this.entry.set(undefined);
      this.name.set('');
      this.description.set('');
      this.deviceType.set('');
      this.fileName.set('');
      this.useTemplate(this.startFrom());
      return;
    }
    this.isNew.set(false);
    try {
      const { data } = await this.inventory.detail(id);
      if (this.route.snapshot.paramMap.get('id') !== id) {
        return;
      }
      this.entry.set(data);
      this.name.set(data['name'] ?? '');
      this.description.set(data['description'] ?? '');
      this.deviceType.set(data['deviceType'] ?? '');
      this.fileName.set(nameOf(data['configurationType']) ?? '');
      this.content.set(data['url'] ? await this.repository.getBinaryText(data['url'], { allowExternal: false }) : '');
      // A file the form cannot read opens as TOML, where the problem is marked.
      if (this.result().config) {
        this.showForm();
      } else {
        this.showToml();
      }
    } catch (e) {
      this.alert.addServerFailure(e);
    }
  }

  useTemplate(protocol: ProtocolId): void {
    this.content.set(emitToml(newConfig(protocol)));
  }

  showForm(): void {
    this.editor = undefined;
    this.view.set('form');
  }

  showToml(): void {
    this.view.set('toml');
  }

  /**
   * A form edit, written into the file's text so its comments and layout stay. When the edit
   * cannot be written that way, the whole file is rewritten in the standard layout, after asking
   * if that drops anything.
   */
  async onFormChange(config: Config): Promise<void> {
    const text = this.content();
    const kept = writeKeepingLayout(text, config);
    if (kept !== undefined) {
      this.content.set(kept);
    } else {
      const current = this.result().config;
      const losesNothing = !!current && emitToml(current) === text;
      if (!losesNothing && !(await this.confirmRewrite(commentLines(text)))) {
        return;
      }
      this.content.set(emitToml(config));
    }
    // "Start from" names the file's protocol, also after the form switched it.
    const protocol = config['connector']?.protocol;
    if ((PROTOCOL_IDS as readonly string[]).includes(protocol)) {
      this.startFrom.set(protocol);
    }
  }

  private async confirmRewrite(comments: number): Promise<boolean> {
    const drops = comments > 0 ? ` and drops its ${comments} comment ${comments === 1 ? 'line' : 'lines'}` : '';
    return this.modal
      .confirm(
        gettext('Rewrite the file?'),
        `This change cannot be written into the file as it is. Applying it rewrites the whole file in the standard layout${drops}. Cancel keeps the file unchanged.`,
        Status.WARNING,
        { ok: gettext('Rewrite the file'), cancel: gettext('Cancel') }
      )
      .then(ok => ok !== false)
      .catch(() => false);
  }

  async onEditorInit(editor: Monaco.editor.IStandaloneCodeEditor): Promise<void> {
    this.editor = editor;
    const monaco = await loadMonacoEditor();
    this.monaco = monaco;
    if (!monaco.languages.getLanguages().some(l => l.id === TOML_LANGUAGE_ID)) {
      monaco.languages.register({ id: TOML_LANGUAGE_ID, extensions: ['.toml'] });
      monaco.languages.setMonarchTokensProvider(TOML_LANGUAGE_ID, tomlLanguage as Monaco.languages.IMonarchLanguage);
    }
    const model = editor.getModel();
    if (model) {
      monaco.editor.setModelLanguage(model, TOML_LANGUAGE_ID);
    }
    this.applyMarkers(this.result());
  }

  private applyMarkers(result: ValidationResult): void {
    const model = this.editor?.getModel();
    if (!this.monaco || !model) {
      return;
    }
    const syntax = result.syntax;
    this.monaco.editor.setModelMarkers(
      model,
      MARKER_OWNER,
      syntax
        ? [
            {
              startLineNumber: syntax.line,
              startColumn: syntax.column,
              endLineNumber: syntax.line,
              endColumn: syntax.column + 1,
              message: syntax.message,
              severity: this.monaco.MarkerSeverity.Error,
            },
          ]
        : []
    );
  }

  cancel(): void {
    void this.router.navigate(['/ot-connectors']);
  }

  async save(): Promise<void> {
    if (!this.canSave()) {
      return;
    }
    this.saving.set(true);
    const name = normalizeName(this.fileName());
    try {
      const file = new File([this.content()], `${name}.toml`, { type: 'text/plain' });
      await this.repository.create(
        {
          version: this.name().trim(),
          description: this.description().trim(),
          deviceType: this.deviceType().trim() || undefined,
          configurationType: typeOf(name),
          binary: { file },
        },
        RepositoryType.CONFIGURATION,
        this.entry()
      );
      this.alert.success(gettext('Connector file saved.'));
      void this.router.navigate(['/ot-connectors']);
    } catch (e) {
      this.alert.addServerFailure(e);
    } finally {
      this.saving.set(false);
    }
  }
}
