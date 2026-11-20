import { Component, Injectable, ViewEncapsulation, computed, inject, input, output, signal } from '@angular/core';
import { CoreModule } from '@c8y/ngx-components';
import { PopoverDirective } from 'ngx-bootstrap/popover';

import {
  PROTOCOL_IDS,
  SCHEMA,
  coerce,
  getAt,
  protocolInfo,
  setAt,
  type Config,
} from '../generated/config-builder';
import { AGGREGATES, OUTPUTS, POLICIES, REPORT_FIELDS } from '../generated/form-options';
import { HELP, helpForPath } from '../generated/help';
import {
  addDevice,
  addPoint,
  displayValue,
  extraKeys,
  pointSummary,
  removeDevice,
  removePoint,
  shapeFields,
  splitList,
  switchProtocol,
  type FieldKind,
} from '../shared/form-model';
import { type FieldProblem } from '../shared/validation';

// The form view of a connector file: the sections and fields of the docs site's config builder
// (site/src/components/tools/ConfigBuilder.tsx), on the same model, options and help texts.

const POINT = SCHEMA.$defs.point.properties;
const DEVICE = SCHEMA.$defs.device.properties;
const CONNECTOR = SCHEMA.properties.connector.properties;
const DEPRECATED_OUTPUT: Record<string, string> = { samples: 'single', measurements: 'group' };

/** The keys each level of the form has fields for; others are kept and listed. */
const KNOWN = {
  file: ['connector', 'mqtt', 'connection', 'device'],
  connector: ['protocol', 'service_name', 'poll_interval', 'sampling_interval', 'log_level', 'output', 'max_inflight', 'max_inflight_total', 'report', 'measurement', 'grouping', 'batch'],
  mqtt: ['host', 'port'],
  device: ['name', 'poll_interval', 'type', 'default_mode', 'max_inflight', 'points_from', 'protocol_address', 'point'],
  point: ['id', 'datatype', 'access', 'unit', 'poll_interval', 'sampling_interval', 'subscribe', 'address', 'report', 'grouping', 'batch'],
};

interface FormSource {
  config: () => Config;
  problems: () => readonly FieldProblem[];
  readOnly: () => boolean;
  apply: (next: Config) => void;
}

/** The file being edited, shared by the form's parts. The form binds it before its parts exist. */
@Injectable()
export class FormState {
  private source: FormSource = { config: () => ({}), problems: () => [], readOnly: () => false, apply: () => undefined };
  readonly config = computed<Config>(() => this.source.config());
  readonly errors = computed<ReadonlyMap<string, string>>(() => new Map(this.source.problems().map(p => [p.path, p.message])));
  readonly readOnly = computed(() => this.source.readOnly());
  readonly protocol = computed<string>(() => this.config()['connector']?.protocol ?? '');

  bind(source: FormSource): void {
    this.source = source;
  }

  value(path: string): any {
    return getAt(this.config(), path);
  }

  error(path: string): string | undefined {
    return this.errors().get(path);
  }

  help(path: string): string | undefined {
    return helpForPath(path, this.protocol());
  }

  set(path: string, value: unknown): void {
    this.edit(config => setAt(config, path, value));
  }

  edit(change: (config: Config) => Config): void {
    if (!this.readOnly()) {
      this.source.apply(change(this.config()));
    }
  }
}

/** A help button whose text opens on focus. */
@Component({
  selector: 'tdot-help',
  imports: [PopoverDirective],
  template: `
    @if (text()) {
      <button
        type="button"
        class="btn-help"
        [attr.aria-label]="'About ' + label()"
        [popover]="text()!"
        placement="top"
        triggers="focus"
        container="body"
      ></button>
    }
  `,
})
export class FormHelpComponent {
  readonly text = input<string | undefined>();
  readonly label = input('');
}

export type FormFieldKind = FieldKind | 'toggle' | 'list';

/** One field of the form: text, number, choice, checkbox, toggle with a default, or a list. */
@Component({
  selector: 'tdot-field',
  imports: [CoreModule, FormHelpComponent],
  host: { class: 'tdot-field' },
  template: `
    @switch (kind()) {
      @case ('check') {
        <label class="c8y-checkbox tdot-check" [title]="path()">
          <input type="checkbox" [checked]="value() === true" (change)="setChecked($event)" />
          <span></span>
          <span><code>{{ name() }}</code></span>
          <tdot-help [text]="state.help(path())" [label]="name()" />
        </label>
      }
      @case ('toggle') {
        <label class="c8y-checkbox tdot-check" [title]="path()">
          <input type="checkbox" [checked]="value() ?? fallback()" (change)="setToggle($event)" />
          <span></span>
          <span><code>{{ name() }}</code></span>
          <tdot-help [text]="state.help(path())" [label]="name()" />
        </label>
      }
      @default {
        <div class="form-group form-group-sm" [class.has-error]="!!error()">
          <label [for]="id()">
            <code>{{ name() }}</code>
            <tdot-help [text]="state.help(path())" [label]="name()" />
          </label>
          @if (kind() === 'choice') {
            <div class="c8y-select-wrapper">
              <select class="form-control" [id]="id()" (change)="setChoice($event)">
                <option value="" [selected]="value() === undefined">—</option>
                @for (o of choices(); track o) {
                  <option [value]="o" [selected]="o === shownChoice()">{{ o }}</option>
                }
              </select>
            </div>
          } @else {
            <input
              class="form-control"
              type="text"
              spellcheck="false"
              [id]="id()"
              [attr.inputmode]="kind() === 'integer' || kind() === 'number' ? 'decimal' : null"
              [value]="shown()"
              [placeholder]="placeholder() ?? ''"
              (input)="setText($event)"
            />
          }
          @if (hint()) {
            <p class="help-block">{{ hint() }}</p>
          }
          @if (error(); as e) {
            <p class="form-control-feedback-message text-danger">{{ e }}</p>
          }
        </div>
      }
    }
  `,
})
export class FormFieldComponent {
  readonly state = inject(FormState);
  readonly path = input.required<string>();
  readonly label = input<string>();
  readonly kind = input<FormFieldKind>('text');
  readonly options = input<readonly unknown[]>([]);
  readonly placeholder = input<string>();
  readonly hint = input<string>();
  /** For a toggle: the value an absent key means. */
  readonly fallback = input(false);

  readonly name = computed(() => this.label() ?? this.path().split('/').pop() ?? '');
  readonly id = computed(() => `tdot${this.path().replace(/[^a-z0-9]+/gi, '-')}`);
  readonly value = computed(() => this.state.value(this.path()));
  readonly error = computed(() => this.state.error(this.path()));
  /** The options, plus the current value when the file holds one the form does not offer. */
  readonly choices = computed(() => {
    const options = this.options().map(String);
    const current = this.value();
    return current !== undefined && !options.includes(String(current)) ? [...options, String(current)] : options;
  });
  readonly shownChoice = computed(() => (this.value() === undefined ? '' : String(this.value())));

  /** What is being typed, kept while it still means the stored value ("1." while typing 1.5). */
  private readonly draft = signal<string | undefined>(undefined);
  readonly shown = computed(() => {
    const draft = this.draft();
    const value = this.value();
    if (draft !== undefined && JSON.stringify(this.parse(draft)) === JSON.stringify(value)) {
      return draft;
    }
    return this.kind() === 'list' && Array.isArray(value) ? value.join(', ') : displayValue(value);
  });

  setText(event: Event): void {
    const text = (event.target as HTMLInputElement).value;
    this.draft.set(text);
    this.state.set(this.path(), this.parse(text));
  }

  setChoice(event: Event): void {
    const text = (event.target as HTMLSelectElement).value;
    const option = this.options().find(o => String(o) === text);
    this.state.set(this.path(), text === '' ? undefined : (option ?? text));
  }

  setChecked(event: Event): void {
    this.state.set(this.path(), (event.target as HTMLInputElement).checked || undefined);
  }

  setToggle(event: Event): void {
    const on = (event.target as HTMLInputElement).checked;
    this.state.set(this.path(), on === this.fallback() ? undefined : on);
  }

  private parse(text: string): unknown {
    if (this.kind() === 'list') {
      const items = splitList(text);
      return items.length ? items : undefined;
    }
    return coerce(text, this.kind());
  }
}

/** The fields of a protocol table, from the builder's protocol shape. */
@Component({
  selector: 'tdot-shape',
  imports: [FormFieldComponent],
  template: `
    <p class="tdot-subtitle">
      <code>{{ legend() }}</code>
      @if (state.error(base()); as e) {
        <span class="text-danger"> {{ e }}</span>
      }
    </p>
    <div class="tdot-row">
      @for (f of fields(); track f.key) {
        <tdot-field [path]="base() + '/' + f.key" [label]="f.key" [kind]="f.kind" [options]="f.options ?? []" [placeholder]="f.placeholder" />
      } @empty {
        <p class="text-muted small">The form has no fields for this table on this protocol; set them in the TOML view.</p>
      }
    </div>
    @if (extra().length) {
      <p class="text-muted small">Also set, kept as is: @for (k of extra(); track k) {<code>{{ k }}</code> }</p>
    }
  `,
})
export class FormShapeComponent {
  readonly state = inject(FormState);
  readonly base = input.required<string>();
  readonly schema = input<any>();
  readonly legend = input('');
  readonly fields = computed(() => shapeFields(this.schema()));
  readonly extra = computed(() => extraKeys(this.state.value(this.base()), this.fields().map(f => f.key)));
}

/** A `report` table: the reporting policy of the connector or of one point. */
@Component({
  selector: 'tdot-report',
  imports: [FormFieldComponent, FormHelpComponent],
  template: `
    <details class="tdot-details" [open]="state.value(base()) !== undefined">
      <summary>
        <code>report</code>
        <tdot-help [text]="help()" label="report" />
        <span class="text-muted"> {{ level() === 'point' ? "this point's reporting policy, over [connector]'s" : 'reporting policy' }}</span>
      </summary>
      <div class="tdot-row">
        <tdot-field [path]="base() + '/on_change'" kind="check" />
        @for (f of fields; track f.key) {
          <tdot-field
            [path]="base() + '/' + f.key"
            [kind]="f.key === 'deadband' ? 'number' : 'text'"
            [placeholder]="f.key === 'deadband' ? '0.5 or &quot;2%&quot;' : 'off'"
          />
        }
      </div>
    </details>
  `,
})
export class FormReportComponent {
  readonly state = inject(FormState);
  readonly base = input.required<string>();
  readonly level = input<'connector' | 'point'>('connector');
  readonly fields = REPORT_FIELDS;
  readonly help = computed(() => HELP[`${this.level()}.report`]);
}

/** `[connector] measurement`: how grouped output names its measurements. */
@Component({
  selector: 'tdot-measurement',
  imports: [FormFieldComponent, FormHelpComponent],
  template: `
    <details class="tdot-details" [open]="state.value(base()) !== undefined">
      <summary>
        <code>measurement</code>
        <tdot-help [text]="help" label="measurement" />
        <span class="text-muted"> the type and fragment of the measurements</span>
      </summary>
      <div class="tdot-row">
        <tdot-field [path]="base() + '/type'" [placeholder]="state.protocol() || 'protocol'" />
        <tdot-field [path]="base() + '/fragment'" placeholder="the type" />
        <tdot-field [path]="base() + '/point_separator'" placeholder="off" />
      </div>
      @if (state.value(base() + '/group') !== undefined) {
        <p class="text-warning small">
          <code>group</code> is deprecated: it sets the type and the fragment. Set <code>type</code> (and
          <code>fragment</code>) instead.
        </p>
      }
    </details>
  `,
})
export class FormMeasurementComponent {
  readonly state = inject(FormState);
  readonly base = input.required<string>();
  readonly help = HELP['connector.measurement'];
}

/** The `grouping` table of `[connector]` or a point; `owner` is the table that holds it. */
@Component({
  selector: 'tdot-grouping',
  imports: [FormFieldComponent, FormHelpComponent],
  template: `
    <details class="tdot-details" [open]="state.value(base()) !== undefined || ownerTable()['batch'] !== undefined">
      <summary>
        <code>grouping</code>
        <tdot-help [text]="help()" label="grouping" />
        <span class="text-muted"> {{ level() === 'point' ? "this point's grouping, over [connector]'s" : 'which readings share a measurement' }}</span>
      </summary>
      @if (ownerTable()['batch'] !== undefined) {
        <p class="text-warning small">
          <code>batch</code> is the deprecated name of <code>grouping</code>.
          @if (ownerTable()['grouping'] === undefined) {
            <button type="button" class="btn btn-default btn-xs" (click)="renameBatch()">Rename it to grouping</button>
          }
        </p>
      }
      <div class="tdot-row">
        <tdot-field [path]="base() + '/by'" kind="choice" [options]="policies" />
        @if (by() === 'frame') {
          <tdot-field [path]="base() + '/window'" placeholder="500ms" />
        }
        @if (sample()) {
          <tdot-field [path]="base() + '/every'" placeholder="1s" />
          <div class="form-group form-group-sm tdot-field">
            <label [for]="aggregateId()"><code>aggregate</code> <tdot-help [text]="aggregateHelp" label="aggregate" /></label>
            <div class="c8y-select-wrapper">
              <select class="form-control" [id]="aggregateId()" (change)="setAggregate($event)">
                @for (a of aggregates; track a.id) {
                  <option [value]="a.id" [selected]="a.id === aggregate()">{{ a.label }}</option>
                }
              </select>
            </div>
          </div>
        }
      </div>
    </details>
  `,
})
export class FormGroupingComponent {
  readonly state = inject(FormState);
  /** The path of the table that holds `grouping`: `/connector` or a point. */
  readonly owner = input.required<string>();
  readonly level = input<'connector' | 'point'>('connector');
  readonly policies = POLICIES.map(p => p.id);
  readonly aggregates = AGGREGATES;
  readonly aggregateHelp = HELP['grouping.aggregate'];

  readonly base = computed(() => `${this.owner()}/grouping`);
  readonly ownerTable = computed<Config>(() => this.state.value(this.owner()) ?? {});
  readonly help = computed(() => HELP[`${this.level()}.grouping`]);
  readonly by = computed(() => this.state.value(`${this.base()}/by`));
  readonly sample = computed(() => this.by() === 'sample' || (this.by() === undefined && this.state.value(`${this.base()}/aggregate`) !== undefined));
  readonly aggregate = computed(() => {
    const value = JSON.stringify(this.state.value(`${this.base()}/aggregate`) ?? null);
    return AGGREGATES.find(a => JSON.stringify(a.value) === value)?.id ?? 'none';
  });
  readonly aggregateId = computed(() => `tdot${this.base().replace(/[^a-z0-9]+/gi, '-')}-aggregate`);

  setAggregate(event: Event): void {
    const id = (event.target as HTMLSelectElement).value;
    this.state.set(`${this.base()}/aggregate`, AGGREGATES.find(a => a.id === id)?.value ?? undefined);
  }

  /** A deprecated `batch` table: the same policy, with `group_by` named `by`. */
  renameBatch(): void {
    const owner = this.ownerTable();
    const { group_by, ...rest } = owner['batch'] ?? {};
    const grouping = group_by === undefined ? rest : { ...rest, by: group_by };
    const { batch: _, ...others } = owner;
    this.state.set(this.owner(), { ...others, grouping });
  }
}

/**
 * The form view of a connector file. It shows `config` and emits each edit as a whole new
 * configuration; the editor writes that back as TOML.
 */
@Component({
  selector: 'tdot-connector-form',
  imports: [
    CoreModule,
    FormFieldComponent,
    FormHelpComponent,
    FormShapeComponent,
    FormReportComponent,
    FormMeasurementComponent,
    FormGroupingComponent,
  ],
  providers: [FormState],
  template: `
    <fieldset class="tdot-form" [disabled]="readOnly()">
      <div class="tdot-row">
        <div class="form-group form-group-sm tdot-field">
          <label for="tdot-protocol"><code>protocol</code> <tdot-help [text]="protocolHelp" label="protocol" /></label>
          <div class="c8y-select-wrapper">
            <select class="form-control" id="tdot-protocol" (change)="switchTo($event)">
              @for (id of protocols; track id) {
                <option [value]="id" [selected]="id === protocol()">{{ label(id) }}</option>
              }
              @if (protocol() && !protocols.includes($any(protocol()))) {
                <option [value]="protocol()" selected>{{ protocol() }} (not known to the form)</option>
              }
            </select>
          </div>
        </div>
      </div>

      <fieldset class="c8y-fieldset">
        <legend><code>[connector]</code></legend>
        <div class="tdot-row">
          <tdot-field path="/connector/service_name" />
          <tdot-field path="/connector/poll_interval" placeholder="2s" />
          @if (protocol() === 'opcua') {
            <tdot-field path="/connector/sampling_interval" placeholder="poll_interval" />
          }
          <tdot-field path="/connector/log_level" kind="choice" [options]="logLevels" />
        </div>
        <div class="tdot-row">
          <tdot-field path="/connector/output" kind="choice" [options]="outputs()" />
          <tdot-field path="/connector/max_inflight" kind="integer" placeholder="4" />
          <tdot-field path="/connector/max_inflight_total" kind="integer" placeholder="32" />
        </div>
        <tdot-report base="/connector/report" level="connector" />
        @if (deprecatedOutput(); as replacement) {
          <p class="text-warning small">
            <code>output = "{{ connector()['output'] }}"</code> is deprecated: it means <code>"{{ replacement }}"</code>.
            <button type="button" class="btn btn-default btn-xs" (click)="state.set('/connector/output', replacement)">
              Use output = "{{ replacement }}"
            </button>
          </p>
        }
        @if (grouped()) {
          <tdot-measurement base="/connector/measurement" />
        }
        @if (grouped() || connector()['grouping'] || connector()['batch']) {
          <tdot-grouping owner="/connector" level="connector" />
        }
        @if (extra('/connector', known.connector).length) {
          <p class="text-muted small">Also set, kept as is: @for (k of extra('/connector', known.connector); track k) {<code>{{ k }}</code> }</p>
        }
      </fieldset>

      <details class="tdot-details c8y-fieldset">
        <summary><code>[mqtt]</code> <span class="text-muted">{{ mqttSummary() }}</span></summary>
        <div class="tdot-row">
          <tdot-field path="/mqtt/host" placeholder="127.0.0.1" />
          <tdot-field path="/mqtt/port" kind="integer" placeholder="1883" />
        </div>
      </details>

      @if (hasConnection()) {
        <details class="tdot-details c8y-fieldset" [open]="hasErrorUnder('/connection')">
          <summary><code>[connection]</code> <span class="text-muted">shared connection settings</span></summary>
          <tdot-shape base="/connection" [schema]="info().connection" legend="shared connection settings" />
        </details>
      }

      @for (device of devices(); track $index; let d = $index) {
        <fieldset class="c8y-fieldset tdot-device">
          <legend><code>[[device]]</code> {{ device['name'] || '#' + (d + 1) }}</legend>
          <div class="tdot-row">
            <tdot-field [path]="'/device/' + d + '/name'" />
            <tdot-field [path]="'/device/' + d + '/poll_interval'" placeholder="inherit" />
          </div>
          <details class="tdot-details" [open]="hasDeviceError(d)">
            <summary>Device settings <span class="text-muted">type, default_mode, max_inflight, points_from, protocol_address</span></summary>
            <div class="tdot-row">
              <tdot-field [path]="'/device/' + d + '/type'" />
              <tdot-field [path]="'/device/' + d + '/default_mode'" kind="choice" [options]="defaultModes" />
              <tdot-field [path]="'/device/' + d + '/max_inflight'" kind="integer" placeholder="inherit" />
            </div>
            <tdot-field [path]="'/device/' + d + '/points_from'" kind="list" hint="Point libraries, comma-separated (for example demo-sim)" />
            <tdot-shape [base]="'/device/' + d + '/protocol_address'" [schema]="info().protocol_address" legend="protocol_address" />
          </details>

          <p class="tdot-subtitle">Points</p>
          @for (point of points(device); track $index; let p = $index) {
            <details class="tdot-details tdot-point" [open]="hasErrorUnder('/device/' + d + '/point/' + p + '/')">
              <summary>
                <code>{{ point['id'] || 'point ' + (p + 1) }}</code>
                <span class="text-muted"> {{ summary(device, point) }}</span>
              </summary>
              <div class="tdot-row">
                <tdot-field [path]="pointPath(d, p, 'id')" />
                <tdot-field [path]="pointPath(d, p, 'datatype')" kind="choice" [options]="datatypes" />
                <tdot-field [path]="pointPath(d, p, 'access')" kind="choice" [options]="accesses" />
                <tdot-field [path]="pointPath(d, p, 'unit')" />
              </div>
              <div class="tdot-row">
                <tdot-field [path]="pointPath(d, p, 'poll_interval')" placeholder="inherit" />
                @if (protocol() === 'opcua') {
                  <tdot-field [path]="pointPath(d, p, 'sampling_interval')" placeholder="inherit" />
                  <tdot-field [path]="pointPath(d, p, 'subscribe')" kind="toggle" [fallback]="true" />
                }
              </div>
              <tdot-shape [base]="pointPath(d, p, 'address')" [schema]="info().address" legend="address" />
              <tdot-report [base]="pointPath(d, p, 'report')" level="point" />
              @if (grouped()) {
                <tdot-grouping [owner]="'/device/' + d + '/point/' + p" level="point" />
              }
              @if (extra('/device/' + d + '/point/' + p, known.point).length) {
                <p class="text-muted small">
                  Also set, kept as is: @for (k of extra('/device/' + d + '/point/' + p, known.point); track k) {<code>{{ k }}</code> }
                </p>
              }
              <button type="button" class="btn btn-default btn-xs" (click)="state.edit(removePointAt(d, p))">Remove point</button>
            </details>
          }
          @if (extra('/device/' + d, known.device).length) {
            <p class="text-muted small">Also set, kept as is: @for (k of extra('/device/' + d, known.device); track k) {<code>{{ k }}</code> }</p>
          }
          <div class="tdot-actions">
            <button type="button" class="btn btn-default btn-sm" (click)="state.edit(addPointTo(d))">
              <i c8yIcon="plus-circle"></i> Add point
            </button>
            <button type="button" class="btn btn-default btn-sm" (click)="state.edit(removeDeviceAt(d))">Remove device</button>
          </div>
        </fieldset>
      }
      <div class="tdot-actions">
        <button type="button" class="btn btn-default btn-sm" (click)="state.edit(addDeviceFn)">
          <i c8yIcon="plus-circle"></i> Add device
        </button>
      </div>
      @if (extra('', known.file).length) {
        <p class="text-muted small">Also set, kept as is: @for (k of extra('', known.file); track k) {<code>{{ k }}</code> }</p>
      }
    </fieldset>
  `,
  // Not encapsulated: the rules reach the form's parts, and every selector starts with .tdot-form.
  encapsulation: ViewEncapsulation.None,
  styles: [
    `
      .tdot-form {
        border: 0;
        margin: 0;
        padding: 0;
        min-width: 0;
      }
      .tdot-form .tdot-row {
        display: flex;
        flex-wrap: wrap;
        gap: 0 16px;
        align-items: flex-start;
      }
      .tdot-form .tdot-row > .tdot-field {
        flex: 1 1 160px;
        max-width: 280px;
      }
      .tdot-form .tdot-check {
        display: flex;
        align-items: center;
        gap: 4px;
        padding-top: 26px;
        margin-bottom: 12px;
      }
      .tdot-form .tdot-details {
        margin: 4px 0 8px;
      }
      .tdot-form .tdot-details > summary {
        cursor: pointer;
        padding: 4px 0;
        list-style: none;
      }
      .tdot-form .tdot-details > summary::-webkit-details-marker {
        display: none;
      }
      .tdot-form .tdot-details > summary::before {
        content: '▸';
        display: inline-block;
        width: 1em;
      }
      .tdot-form .tdot-details[open] > summary::before {
        content: '▾';
      }
      .tdot-form .tdot-point {
        border-left: 2px solid var(--c8y-root-component-border-color, #ddd);
        padding-left: 12px;
      }
      .tdot-form .tdot-subtitle {
        margin: 12px 0 4px;
        font-weight: 600;
      }
      .tdot-form .tdot-actions {
        display: flex;
        gap: 8px;
        margin: 8px 0 16px;
      }
    `,
  ],
})
export class ConnectorFormComponent {
  readonly state = inject(FormState);
  readonly config = input.required<Config>();
  readonly problems = input<readonly FieldProblem[]>([], { alias: 'errors' });
  readonly readOnly = input(false);
  readonly configChange = output<Config>();

  readonly protocols = PROTOCOL_IDS;
  readonly protocolHelp = HELP['connector.protocol'];
  readonly logLevels = CONNECTOR.log_level.enum as string[];
  readonly datatypes = POINT.datatype.enum as string[];
  readonly accesses = POINT.access.enum as string[];
  readonly defaultModes = DEVICE.default_mode.enum as string[];
  readonly known = KNOWN;

  readonly protocol = this.state.protocol;
  readonly connector = computed<Config>(() => this.config()['connector'] ?? {});
  readonly info = computed(() => protocolInfo(this.protocol()));
  readonly devices = computed<Config[]>(() => (Array.isArray(this.config()['device']) ? this.config()['device'] : []));
  readonly grouped = computed(() => ['group', 'measurements'].includes(this.connector()['output']));
  readonly deprecatedOutput = computed(() => DEPRECATED_OUTPUT[this.connector()['output']]);
  readonly outputs = computed(() => (this.deprecatedOutput() ? [...OUTPUTS, this.connector()['output']] : OUTPUTS));
  readonly hasConnection = computed(
    () => Object.keys(this.info()?.connection?.properties ?? {}).length > 0 || this.config()['connection'] !== undefined
  );
  readonly mqttSummary = computed(() => `${this.config()['mqtt']?.host ?? '127.0.0.1'}:${this.config()['mqtt']?.port ?? 1883}`);

  readonly addDeviceFn = (config: Config) => addDevice(config);

  constructor() {
    this.state.bind({
      config: () => this.config(),
      problems: () => this.problems(),
      readOnly: () => this.readOnly(),
      apply: next => this.configChange.emit(next),
    });
  }

  label(id: string): string {
    const info = protocolInfo(id);
    return `${info.label}${info.preview ? ' (preview)' : ''}`;
  }

  points(device: Config): Config[] {
    return Array.isArray(device['point']) ? device['point'] : [];
  }

  pointPath(d: number, p: number, key: string): string {
    return `/device/${d}/point/${p}/${key}`;
  }

  summary(device: Config, point: Config): string {
    return pointSummary(this.config(), device, point);
  }

  extra(path: string, known: readonly string[]): string[] {
    return extraKeys(path ? getAt(this.config(), path) : this.config(), known);
  }

  hasErrorUnder(prefix: string): boolean {
    return this.problems().some(p => p.path.startsWith(prefix));
  }

  hasDeviceError(d: number): boolean {
    return this.problems().some(p => p.path.startsWith(`/device/${d}/`) && !p.path.includes('/point/'));
  }

  switchTo(event: Event): void {
    const protocol = (event.target as HTMLSelectElement).value;
    this.state.edit(config => switchProtocol(config, protocol));
  }

  addPointTo(d: number) {
    return (config: Config) => addPoint(config, d);
  }

  removePointAt(d: number, p: number) {
    return (config: Config) => removePoint(config, d, p);
  }

  removeDeviceAt(d: number) {
    return (config: Config) => removeDevice(config, d);
  }
}
