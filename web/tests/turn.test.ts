import assert from 'node:assert/strict'
import test from 'node:test'

import { applyTurnEvent, isTerminal, parseReadiness, parseTurn } from '../src/lib/voice/turn.ts'
import type { Turn } from '../src/lib/voice/turn.ts'
import { canAutoSpeak, localVoices } from '../src/lib/voice/speech.ts'

function event(turn: Turn | null, name: string, data: Record<string, unknown> = {}) {
  return applyTurnEvent(turn, { event: name, data: JSON.stringify({ turn_id: 'web-1', ...data }) })!
}
function review() {
  return event(event(null, 'turn.created'), 'transcript.ready', { text: 'Original question' })
}

test('transcription requires approval; TUI accepted wording replaces local question', () => {
  let turn = review()
  assert.equal(turn.status, 'awaiting_review')
  assert.equal(turn.approved_text, null)
  assert.equal(turn.reply, '')
  assert.throws(() => event(turn, 'reply.started'), /without an approved question/)
  turn = event(turn, 'question.approved', { text: 'Exact TUI correction 👋' })
  assert.equal(turn.approved_text, 'Exact TUI correction 👋')
  turn = event(turn, 'reply.started')
  turn = event(turn, 'reply.delta', { text: 'Live ' })
  turn = event(turn, 'reply.delta', { text: 'reply' })
  assert.equal(turn.reply, 'Live reply')
  turn = event(turn, 'reply.completed', { text: 'Authoritative full reply' })
  assert.equal(turn.reply, 'Authoritative full reply')
  assert.equal(isTerminal(turn), true)
})

test('other turn IDs and unknown/test events cannot update a web turn', () => {
  const turn = review()
  assert.equal(event(turn, 'question.approved', { turn_id: 'tui-test', text: 'Private test' }), turn)
  assert.equal(applyTurnEvent(turn, { event: 'test.reply', data: 'not parsed' }), turn)
  assert.throws(() => event(null, 'reply.delta', { text: 'wrong stream' }), /did not identify/)
})

test('approval is frozen; duplicate approval cannot erase incremental reply', () => {
  let turn = event(review(), 'question.approved', { text: 'Approved' })
  turn = event(turn, 'reply.delta', { text: 'partial' })
  assert.equal(event(turn, 'question.approved', { text: 'Approved' }), turn)
  assert.throws(() => event(turn, 'question.approved', { text: 'Different' }), /already approved/)
})

test('failed and cancelled turns preserve partial text but ignore late output', () => {
  let turn = event(review(), 'question.approved', { text: 'Question' })
  turn = event(turn, 'reply.delta', { text: 'partial' })
  for (const name of ['turn.cancelled', 'reply.failed', 'turn.failed']) {
    const terminal = event(turn, name, { error: { code: 'busy', message: 'Runtime busy' } })
    assert.equal(isTerminal(terminal), true)
    assert.equal(terminal.reply, 'partial')
    assert.equal(event(terminal, 'reply.completed', { text: 'late' }), terminal)
    assert.equal(event(terminal, 'reply.delta', { text: 'late' }), terminal)
  }
})

test('recovery parses authoritative text/status and unavailable timings without fabricated zeros', () => {
  const recovered = parseTurn({ turn_id: 'web-1', status: 'completed', transcript: 'Original', approved_text: 'Corrected', reply: 'Saved result', error: null, timings: { transcription_ms: 123, power: null } })
  assert.equal(recovered.approved_text, 'Corrected')
  assert.deepEqual(recovered.timings, { transcription_ms: 123, power: null })
  assert.equal(canAutoSpeak(false, true, recovered.reply), false)
  assert.equal(canAutoSpeak(true, false, recovered.reply), false)
  assert.equal(canAutoSpeak(true, true, recovered.reply), true)
  assert.throws(() => parseTurn({ turn_id: 'x', status: 'unknown' }), /Unknown/)
})

test('readiness reads only model badges and local-only voices have no remote fallback', () => {
  assert.deepEqual(parseReadiness({ stt: { name: 'Whisper', ready: true }, reply: { name: 'Local chat', ready: false }, history: ['not consumed'], current_turn: 'not consumed', role: 'not consumed' }), { stt: { name: 'Whisper', ready: true }, reply: { name: 'Local chat', ready: false } })
  const voices = [{ name: 'remote', localService: false }, { name: 'OS', localService: true }]
  assert.deepEqual(localVoices(voices), [voices[1]])
  assert.deepEqual(localVoices([voices[0]]), [])
})
