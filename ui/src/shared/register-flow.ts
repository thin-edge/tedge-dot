/**
 * The order of steps that registers a connector file on a gateway, without Angular or the
 * platform's services, so it is unit-tested with fakes. ConnectorOperationsService provides the
 * real steps.
 */
import { typeOf } from './naming';
import { planRegistration } from './registration';
import { type WaitOptions, waitFor } from './wait';

/** The platform calls the flow needs, for one gateway. */
export interface RegistrationSteps {
  /** Whether the gateway offers the type now (reads the device again). */
  offersType(): Promise<boolean>;
  /** The event id of the newest snapshot of the list of managed files, if there is one. */
  latestSnapshotId(): Promise<string | undefined>;
  /** Asks the gateway for a snapshot of the list, and waits until the operation succeeds. */
  requestSnapshot(): Promise<void>;
  /** The newest snapshot of the list, with its event id and text. */
  latestSnapshot(): Promise<{ id: string; text: string } | undefined>;
  /** Sends the new list to the gateway and waits until the operation succeeds. */
  sendList(text: string): Promise<void>;
}

export class RegistrationCancelled extends Error {
  constructor() {
    super('Cancelled.');
  }
}

export interface RegistrationOptions {
  confirm: (block: string) => Promise<boolean>;
  progress: (step: string) => void;
  snapshotWait?: WaitOptions;
  typeWait?: WaitOptions;
}

/**
 * Registers connector file `name`: a fresh snapshot of the list, the append-only edit, the
 * user's confirmation, the new list, then waiting until the gateway offers the type. Sends
 * nothing when the gateway already offers it, when the path or type is taken, or when the user
 * cancels.
 */
export async function registerConnectorFile(
  steps: RegistrationSteps,
  name: string,
  options: RegistrationOptions
): Promise<'offered' | 'registered'> {
  const type = typeOf(name);
  if (await steps.offersType()) {
    return 'offered';
  }
  options.progress('Getting the list of managed files from the gateway');
  const before = await steps.latestSnapshotId();
  await steps.requestSnapshot();
  const snapshot = await waitFor(
    async () => {
      const latest = await steps.latestSnapshot();
      return latest && latest.id !== before ? latest : undefined;
    },
    'the snapshot of the list of managed files',
    options.snapshotWait ?? { timeoutMs: 60 * 1000 }
  );

  const plan = planRegistration(snapshot.text, name);
  if (plan.kind === 'conflict') {
    throw new Error(plan.message);
  }
  if (plan.kind === 'append') {
    if (!(await options.confirm(plan.block))) {
      throw new RegistrationCancelled();
    }
    options.progress('Sending the extended list to the gateway');
    await steps.sendList(plan.text);
  }
  options.progress(`Waiting for the gateway to offer ${type}`);
  await waitFor(async () => ((await steps.offersType()) ? true : undefined), `the gateway to offer ${type}`, options.typeWait ?? { timeoutMs: 2 * 60 * 1000 });
  return 'registered';
}
