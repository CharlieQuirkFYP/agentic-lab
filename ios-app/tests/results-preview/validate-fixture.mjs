import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildResultsPresentation, projectObservations } from '../../../shared/phone-benchmarks/index.ts';
import { conversationID, snapshotFor } from './fixtures.ts';

const snapshot = snapshotFor(conversationID);
const projection = projectObservations(snapshot.records, { filtered: true, selected_record_keys: snapshot.selected_record_keys });
const current = buildResultsPresentation(projection, { conversation_id: conversationID });
const turn = buildResultsPresentation(projection, { conversation_id: conversationID, turn_id: `${conversationID}-turn-2` });
const operation = buildResultsPresentation(projection, { conversation_id: conversationID, turn_id: `${conversationID}-turn-2`, attempt_id: `${conversationID}-turn-2-reasoning` });
assert.equal(current.turns.length, 3);
assert.equal(current.recordedProcessing.duration_ms, 17_350);
assert.equal(current.recordedProcessing.partial, false);
assert.equal(current.outcomes.cancelled, 1);
assert.equal(current.memory[0].stats.max, 3312);
assert.equal(current.memory[0].sampleCount, 30);
assert.equal(turn.turns.length, 1);
assert.equal(turn.recordedProcessing.duration_ms, 7750);
assert.equal(turn.memory[0].sampleCount, 10);
assert.equal(operation.recordedProcessing.duration_ms, 6400);
assert.equal(operation.memory[0].sampleCount, 5);
assert.equal(operation.memory[0].stats.max, 3056);
assert.equal(current.capabilities.length, 2);
const saved = snapshotFor('illustrative-chat-saved');
const savedView = buildResultsPresentation(projectObservations(saved.records, { filtered: true, selected_record_keys: saved.selected_record_keys }));
assert.equal(savedView.turns.length, 2);
assert.equal(savedView.recordedProcessing.duration_ms, 13_650);
for (const [variant, count, maximum] of [['zero', 30, 0], ['single', 1, 2048], ['missing', 0, null], ['none', null, null]]) {
  const value = snapshotFor(conversationID, undefined, variant);
  const view = buildResultsPresentation(projectObservations(value.records, { filtered: true, selected_record_keys: value.selected_record_keys }));
  const metric = view.metrics.find(item => item.name === 'resident_memory_bytes');
  assert.equal(metric?.stats.count ?? null, count);
  assert.equal(metric?.stats.max ?? null, maximum);
  assert.equal(view.recordedProcessing.duration_ms, 17_350);
}
const result = {
  evidence: 'Deterministic illustrative fixture; no device measurements',
  result: 'passed',
  actualModules: ['shared/phone-benchmarks/projection.ts', 'shared/phone-benchmarks/presentation.ts'],
  current: { turns: 3, processing_ms: 17_350, sampled_memory_max_mib: 3312, memory_samples: 30, cancelled_turns: 1 },
  turn2: { processing_ms: 7750, memory_samples: 10 },
  reply2: { processing_ms: 6400, memory_samples: 5, sampled_memory_max_mib: 3056 },
  saved: { turns: 2, processing_ms: 13_650 },
  edgeCases: ['valid zero remains zero', 'single numeric reading', 'all readings explicitly unavailable', 'no retained memory readings'],
};
const destination = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../.expo/results-validation');
fs.mkdirSync(destination, { recursive: true });
fs.writeFileSync(path.join(destination, 'fixture-validation.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify(result, null, 2));
