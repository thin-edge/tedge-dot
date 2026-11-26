import { type ConfigOperation, missingOnGateway } from './missing-file';

const TYPE = 'tedge-dot-opcua2';
const upload = (creationTime: string, status: string, failureReason?: string, type = TYPE): ConfigOperation => ({
  creationTime,
  status,
  failureReason,
  c8y_UploadConfigFile: { type },
});
const download = (creationTime: string, status: string, type = TYPE): ConfigOperation => ({
  creationTime,
  status,
  c8y_DownloadConfigFile: { type },
});
// As thin-edge.io 2.1 reported it on a tenant, shortened.
const NOT_FOUND =
  "builtin 'builtin:config_snapshot:get' action failed with: config_operation step 'get' failed: Config plugin 'file' error: " +
  'Command execution failed: ... Config file not found: /etc/tedge/plugins/ot/opcua2.toml';

describe('missingOnGateway', () => {
  it('is true when the newest operation of the type is a snapshot that failed for a missing file', () => {
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'FAILED', NOT_FOUND)], TYPE)).toBe(true);
  });

  it('is false for other failures and other states', () => {
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'FAILED', 'Permission denied')], TYPE)).toBe(false);
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'FAILED', "Config plugin 'nosuch' error: Plugin not found")], TYPE)).toBe(false);
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'SUCCESSFUL')], TYPE)).toBe(false);
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'PENDING')], TYPE)).toBe(false);
    expect(missingOnGateway([], TYPE)).toBe(false);
  });

  it('looks at the newest operation of the type only', () => {
    const failed = upload('2026-11-26T09:00:00.000Z', 'FAILED', NOT_FOUND);
    // A send or a snapshot that worked afterwards clears it, in any order of the list.
    expect(missingOnGateway([download('2026-11-26T09:05:00.000Z', 'SUCCESSFUL'), failed], TYPE)).toBe(false);
    expect(missingOnGateway([failed, upload('2026-11-26T09:05:00.000Z', 'SUCCESSFUL')], TYPE)).toBe(false);
    // An older success doesn't.
    expect(missingOnGateway([download('2026-11-26T08:00:00.000Z', 'SUCCESSFUL'), failed], TYPE)).toBe(true);
    // Operations of other types don't count.
    expect(missingOnGateway([failed, download('2026-11-26T09:05:00.000Z', 'SUCCESSFUL', 'tedge-dot-other')], TYPE)).toBe(true);
    expect(missingOnGateway([upload('2026-11-26T09:00:00.000Z', 'FAILED', NOT_FOUND, 'tedge-dot-other')], TYPE)).toBe(false);
  });

  it('compares times, not their spelling', () => {
    const failed = upload('2026-11-26T10:00:00.000+01:00', 'FAILED', NOT_FOUND);
    expect(missingOnGateway([failed, download('2026-11-26T09:30:00.000Z', 'SUCCESSFUL')], TYPE)).toBe(false);
  });
});
