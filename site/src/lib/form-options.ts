// The choices the configuration forms offer: the docs site's config builder and the Device
// Management plugin (ui/, which copies this file at build time). It imports nothing, so it can be
// copied on its own. Tests hold these lists to the contract schema (tests/batching.test.ts).

/** The `output` values the tools offer; "samples" and "measurements" are their deprecated names. */
export const OUTPUTS = ['single', 'group'];

/** Every `grouping.by`, as the playground offers them (a test holds them to the schema). */
export const POLICIES: { id: 'time' | 'frame' | 'sample'; text: string }[] = [
  { id: 'time', text: 'Readings with the same timestamp share a measurement. Nothing is lost, no time is moved. A signal that published nothing at that time is not in the measurement.' },
  { id: 'frame', text: 'Readings within a window share a measurement stamped with its first time. Nothing is lost; a time moves by less than the window. A signal that published nothing in the window is not in it.' },
  { id: 'sample', text: 'Every period, the value each signal holds, stamped with the sample time, whether it changed or not. Readings in between are not sent. A signal with no good value (never read, or in an outage) is left out.' },
];

/** The `grouping.aggregate` choices under "sample" (a test checks they cover every aggregate). */
export const AGGREGATES: { id: string; label: string; value: string | string[] | null }[] = [
  { id: 'none', label: 'none (sample and hold)', value: null },
  { id: 'mean', label: '"mean"', value: 'mean' },
  { id: 'mean_min_max', label: '["mean", "min", "max"]', value: ['mean', 'min', 'max'] },
  { id: 'all', label: '["mean", "min", "max", "last"]', value: ['mean', 'min', 'max', 'last'] },
];

/**
 * The OPC UA `security_policy` names both connector builds know (doc/connectors/opcua-connector-spec.md
 * §3.3), the deprecated SHA-1 ones last. A value may also be a policy's full URI.
 */
export const OPCUA_SECURITY_POLICIES: { name: string; deprecated: boolean }[] = [
  { name: 'None', deprecated: false },
  { name: 'Basic256Sha256', deprecated: false },
  { name: 'Aes128_Sha256_RsaOaep', deprecated: false },
  { name: 'Aes256_Sha256_RsaPss', deprecated: false },
  { name: 'Basic128Rsa15', deprecated: true },
  { name: 'Basic256', deprecated: true },
];

/** The OPC UA `security_mode` values; the connectors read them in any case. */
export const OPCUA_SECURITY_MODES = ['none', 'sign', 'sign_and_encrypt'];

/** The `report` keys after `on_change`, as the config builder offers them (a test holds them to the schema). */
export const REPORT_FIELDS = [
  { key: 'deadband', hint: 'A number, or a percentage like "2%"' },
  { key: 'min_interval', hint: 'At most once per interval, e.g. "30s"' },
  { key: 'max_interval', hint: 'Heartbeat, e.g. "30m"' },
  { key: 'debounce', hint: 'Publish once settled, e.g. "5s"' },
] as const;
