/**
 * Waiting for something the gateway does: an operation to finish, a type to appear.
 *
 * Plain polling with a deadline. Operations on a gateway take seconds, and polling keeps the
 * waiting testable without a realtime connection.
 */

export interface WaitOptions {
  /** Between two checks, in milliseconds. */
  intervalMs?: number;
  /** Give up after this long, in milliseconds. */
  timeoutMs?: number;
  /** For tests. */
  sleep?: (ms: number) => Promise<void>;
  /** For tests. */
  now?: () => number;
}

const defaultSleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

/** Calls `check` until it returns a value other than undefined, or fails after the timeout. */
export async function waitFor<T>(
  check: () => Promise<T | undefined>,
  what: string,
  options: WaitOptions = {}
): Promise<T> {
  const intervalMs = options.intervalMs ?? 2000;
  const timeoutMs = options.timeoutMs ?? 5 * 60 * 1000;
  const sleep = options.sleep ?? defaultSleep;
  const now = options.now ?? Date.now;
  const deadline = now() + timeoutMs;
  for (;;) {
    const value = await check();
    if (value !== undefined) {
      return value;
    }
    if (now() >= deadline) {
      throw new Error(`Timed out waiting for ${what}.`);
    }
    await sleep(intervalMs);
  }
}

/** The part of an operation the waiting needs. */
export interface OperationState {
  /** The platform's OperationStatus: PENDING, EXECUTING, SUCCESSFUL or FAILED. */
  status?: unknown;
  failureReason?: unknown;
}

/**
 * Waits for an operation to succeed. Resolves with the final operation, and rejects with the
 * failure reason when the gateway reports one.
 */
export async function waitForOperation<T extends OperationState>(
  get: () => Promise<T>,
  what: string,
  options: WaitOptions = {}
): Promise<T> {
  const done = await waitFor(
    async () => {
      const op = await get();
      const status = String(op.status);
      return status === 'SUCCESSFUL' || status === 'FAILED' ? op : undefined;
    },
    what,
    options
  );
  if (String(done.status) === 'FAILED') {
    throw new Error(`${what} failed${done.failureReason ? `: ${String(done.failureReason)}` : '.'}`);
  }
  return done;
}
