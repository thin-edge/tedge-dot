import { waitFor, waitForOperation } from './wait';

/** A clock that only moves when the code under test sleeps. */
function fakeTime() {
  let t = 0;
  return { now: () => t, sleep: async (ms: number) => void (t += ms) };
}

describe('waitFor', () => {
  it('returns the first defined value', async () => {
    let calls = 0;
    const value = await waitFor(async () => (++calls === 3 ? 'ready' : undefined), 'x', fakeTime());
    expect(value).toBe('ready');
    expect(calls).toBe(3);
  });

  it('gives up after the timeout', async () => {
    await expect(
      waitFor(async () => undefined, 'the new type', { ...fakeTime(), intervalMs: 1000, timeoutMs: 5000 })
    ).rejects.toThrow('Timed out waiting for the new type.');
  });
});

describe('waitForOperation', () => {
  it('resolves when the operation succeeds', async () => {
    const states = ['PENDING', 'EXECUTING', 'SUCCESSFUL'];
    const op = await waitForOperation(async () => ({ status: states.shift() }), 'op', fakeTime());
    expect(op.status).toBe('SUCCESSFUL');
  });

  it('rejects with the failure reason', async () => {
    await expect(
      waitForOperation(
        async () => ({ status: 'FAILED', failureReason: 'Permission denied' }),
        'Registering line-3',
        fakeTime()
      )
    ).rejects.toThrow('Registering line-3 failed: Permission denied');
  });
});
