import assert from 'node:assert/strict';
import test from 'node:test';
import { summarizeObservations } from '../src/lib/metricObservations.ts';

test('phone observations retain separate sources/scopes and explicit unavailable values', () => {
  const base = { name: 'ram_usage_bytes', unit: 'bytes', scope: 'process', source: 'native.ios' };
  const groups = summarizeObservations([{ events: [
    { ...base, value: 0 }, { ...base, value: 100 },
    { ...base, scope: 'system', value: 1000 },
    { ...base, source: 'sysinfo', value: 2000 },
    { ...base, name: 'whole_device_power_watts', unit: 'watts', scope: 'device', value: null },
    { ...base, name: 'model_name', unit: 'status', value: 'test model' },
  ] }]);
  assert.equal(groups.length, 4);
  assert.equal(groups.find(g => g.scope === 'process' && g.source === 'native.ios')?.mean, 50);
  assert.equal(groups.find(g => g.scope === 'system')?.mean, 1000);
  assert.equal(groups.find(g => g.source === 'sysinfo')?.mean, 2000);
  const power = groups.find(g => g.name === 'whole_device_power_watts');
  assert.equal(power?.mean, null);
  assert.equal(power?.unavailable, 1);
  assert.equal(power?.count, 0);
});
