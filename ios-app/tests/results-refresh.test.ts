import assert from 'node:assert/strict';
import test from 'node:test';
import { subscribeResultsRefresh } from '../src/resultsRefresh.ts';

test('visible archive changes coalesce to one refresh per second and hidden work stops', () => {
  let clock = 0, calls = 0, subscribed = false;
  let notify = () => {};
  let pending: { callback: () => void; at: number } | null = null;
  const stop = subscribeResultsRefresh({
    subscribe: callback => { subscribed = true; notify = callback; return () => { subscribed = false; }; },
    refresh: () => calls++, now: () => clock,
    schedule: (callback, delay) => {
      assert.equal(pending, null); pending = { callback, at: clock + delay };
      return 1 as unknown as ReturnType<typeof setTimeout>;
    }, cancel: () => { pending = null; },
  });
  assert.equal(calls, 1);
  for (clock = 1; clock < 1000; clock++) notify();
  assert.equal(calls, 1);
  assert.equal(pending!.at, 1000);
  const fire = pending!.callback; pending = null; fire();
  assert.equal(calls, 2);
  clock = 1001; notify();
  stop();
  assert.equal(subscribed, false); assert.equal(pending, null);
  notify(); assert.equal(pending, null); assert.equal(calls, 2);
});

test('a new visible subscription reads immediately after being hidden', () => {
  let calls = 0;
  const options = { subscribe: () => () => {}, refresh: () => calls++ };
  const first = subscribeResultsRefresh(options); first();
  const second = subscribeResultsRefresh(options); second();
  assert.equal(calls, 2);
});
