import { RegistrationCancelled, type RegistrationSteps, registerConnectorFile } from './register-flow';
import { registrationBlock } from './registration';

const LIST = "[[files]]\npath = '/etc/tedge/tedge.toml'\ntype = 'tedge.toml'\n";

/** A gateway that answers like thin-edge.io: a new snapshot per request, the type once registered. */
function fakeGateway(list: string, offered = false) {
  const calls: string[] = [];
  let snapshots = 0;
  let current = list;
  let offers = offered;
  const steps: RegistrationSteps = {
    offersType: async () => {
      calls.push('offersType');
      return offers;
    },
    latestSnapshotId: async () => (snapshots ? `s${snapshots}` : undefined),
    requestSnapshot: async () => {
      calls.push('requestSnapshot');
      snapshots++;
    },
    latestSnapshot: async () => (snapshots ? { id: `s${snapshots}`, text: current } : undefined),
    sendList: async text => {
      calls.push('sendList');
      current = text;
      offers = true;
    },
  };
  return { steps, calls, list: () => current };
}

const quick = { intervalMs: 1, timeoutMs: 50, sleep: async () => undefined };

describe('registerConnectorFile', () => {
  it('appends the entry after confirmation and waits for the type', async () => {
    const gw = fakeGateway(LIST);
    const confirmed: string[] = [];
    const result = await registerConnectorFile(gw.steps, 'line-3', {
      confirm: async block => (confirmed.push(block), true),
      progress: () => undefined,
      snapshotWait: quick,
      typeWait: quick,
    });
    expect(result).toBe('registered');
    expect(confirmed).toEqual([registrationBlock('line-3')]);
    expect(gw.list()).toBe(LIST + '\n' + registrationBlock('line-3'));
    expect(gw.calls).toEqual(['offersType', 'requestSnapshot', 'sendList', 'offersType']);
  });

  it('does nothing when the gateway offers the type already', async () => {
    const gw = fakeGateway(LIST, true);
    const result = await registerConnectorFile(gw.steps, 'line-3', {
      confirm: async () => true,
      progress: () => undefined,
    });
    expect(result).toBe('offered');
    expect(gw.calls).toEqual(['offersType']);
  });

  it('sends nothing when the user cancels', async () => {
    const gw = fakeGateway(LIST);
    await expect(
      registerConnectorFile(gw.steps, 'line-3', {
        confirm: async () => false,
        progress: () => undefined,
        snapshotWait: quick,
      })
    ).rejects.toBeInstanceOf(RegistrationCancelled);
    expect(gw.calls).not.toContain('sendList');
  });

  it('sends nothing when the path is registered under another type', async () => {
    const gw = fakeGateway(LIST + "\n[[files]]\npath = '/etc/tedge/plugins/ot/line-3.toml'\ntype = 'line3'\n");
    await expect(
      registerConnectorFile(gw.steps, 'line-3', { confirm: async () => true, progress: () => undefined, snapshotWait: quick })
    ).rejects.toThrow('already registered as the type "line3"');
    expect(gw.calls).not.toContain('sendList');
  });

  it('waits for the type without sending when the list has the entry but the gateway does not offer it yet', async () => {
    const gw = fakeGateway(LIST + '\n' + registrationBlock('line-3'));
    let checks = 0;
    const steps = { ...gw.steps, offersType: async () => ++checks > 2 };
    await registerConnectorFile(steps, 'line-3', {
      confirm: async () => true,
      progress: () => undefined,
      snapshotWait: quick,
      typeWait: quick,
    });
    expect(gw.calls).toEqual(['requestSnapshot']);
  });

  it('uses a snapshot newer than the one before the request', async () => {
    const gw = fakeGateway(LIST);
    let served = 0;
    const steps: RegistrationSteps = {
      ...gw.steps,
      latestSnapshotId: async () => 'old',
      latestSnapshot: async () => (++served < 3 ? { id: 'old', text: 'not valid toml [[[' } : { id: 'new', text: LIST }),
    };
    await registerConnectorFile(steps, 'line-3', {
      confirm: async () => true,
      progress: () => undefined,
      snapshotWait: quick,
      typeWait: quick,
    });
    expect(served).toBe(3);
  });
});
