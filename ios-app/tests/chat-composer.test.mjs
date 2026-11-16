import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const screen = readFileSync(new URL('../app/index.tsx', import.meta.url), 'utf8');
const approval = screen.match(/async function approve\(\) \{([\s\S]*?)\n  async function pollTurn/)?.[1];
const eventUpdates = screen.match(/function updateFromEvent\([\s\S]*?\n  async function runTurn/)?.[0];

test('reviewed voice submission uses the acceptance event that clears the composer', () => {
  assert.ok(approval);
  assert.match(approval, /await submitTurn\(turnID, text, updateFromEvent\)/);
  assert.ok(eventUpdates);
  const [beforeTurnUpdate, turnUpdate] = eventUpdates.split('setActive((current) => {');
  assert.match(beforeTurnUpdate, /if \(event\.event === "question\.approved"\) setDraft\(""\)/);
  assert.ok(turnUpdate);
  assert.doesNotMatch(turnUpdate, /setDraft\(/);
  assert.doesNotMatch(approval, /setActive\(/);
});

test('failed voice submission restores the edited draft for retry', () => {
  assert.ok(approval);
  assert.match(approval, /const text = draft\.trim\(\)/);
  assert.match(approval, /catch \(e\) \{\s*setDraft\(text\)/);
});
