/**
 * Whether a gateway lists a file that is not there. thin-edge.io fails a snapshot request for a
 * registered path without a file with "Config file not found: <path>". The newest configuration
 * operation of the type decides, so a send or a snapshot that worked afterwards clears it.
 *
 * Plain TypeScript, without Angular, so it is unit-tested as it is.
 */

/** The part of a `c8y_UploadConfigFile` or `c8y_DownloadConfigFile` operation the check needs. */
export interface ConfigOperation {
  creationTime?: string;
  status?: string;
  failureReason?: string;
  c8y_UploadConfigFile?: { type?: string };
  c8y_DownloadConfigFile?: { type?: string };
}

/** thin-edge.io's text; "Plugin not found", for an unknown plugin, is another failure. */
const NOT_FOUND = /config file not found/i;

function time(operation: ConfigOperation): number {
  const t = Date.parse(operation.creationTime ?? '');
  return Number.isNaN(t) ? 0 : t;
}

/** Whether the newest operation of `type` is a snapshot request that failed for a missing file. */
export function missingOnGateway(operations: readonly ConfigOperation[], type: string): boolean {
  let newest: ConfigOperation | undefined;
  for (const operation of operations) {
    if (operation.c8y_UploadConfigFile?.type !== type && operation.c8y_DownloadConfigFile?.type !== type) {
      continue;
    }
    if (!newest || time(operation) > time(newest)) {
      newest = operation;
    }
  }
  return (
    !!newest?.c8y_UploadConfigFile && newest.status === 'FAILED' && NOT_FOUND.test(newest.failureReason ?? '')
  );
}
