import type { On } from 'claude-code'
import { test, expect } from 'claude-code/testing'

import { pastedRow, pastedSubmit, postToolBatch, rewakeRow, rewakeSubmit, stop, typedRow, typedSubmit } from './captured'

// Every input here is a capture from Claude Code 2.1.301 (tests/captured.ts).
const P = 'herdr-inbox-ui'
const SURFACES = ['terminal', 'desktop'] as const

const engine = (on: On) =>
  on('ui.render', ($, e) => { const { Text } = $.ui.resolve(e); return <Text>ENGINE</Text> })
const sinks = (on: On) => {
  const toasts: string[] = []
  const logs: string[] = []
  const prompts: string[] = []
  on('ui.toast', (_$, e) => { toasts.push(e.text); return { value: undefined } })
  on('ui.log', (_$, e) => { logs.push(e.text); return { value: undefined } })
  on('prompt.submit', (_$, e) => { prompts.push(e.text); return { text: e.text } })
  return { toasts, logs, prompts }
}
const submit = (r: { text: string; origin: { kind: string } }) =>
  ({ text: r.text, wait: false, origin: { ...r.origin } as { kind: 'composer' } })
const userRow = (r: typeof typedRow | typeof pastedRow | typeof rewakeRow, isExpanded = false) => ({
  component: 'UserMessage' as const, requestId: r.requestId,
  props: { ...r.props, origin: { ...r.props.origin }, isExpanded } as never,
})

test('typed answer (composer): compact block, toast', async ($, on) => {
  const { toasts, prompts } = sinks(on)
  await $.prompt.submit(submit(typedSubmit))
  expect(toasts).toEqual(['Operator answered Q16'])
  expect(prompts).toEqual([typedSubmit.text])
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...userRow(typedRow) })
    expect(await ui.find({ type: 'Text', text: /^✓ Operator answered Q16 — Typed one\?$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^sure$/ })).toBeDefined()
    await ui.unmount()
  }
})

test('pasted answer: the id-carrying closing tag is not part of the answer', async ($, on) => {
  const { toasts } = sinks(on)
  await $.prompt.submit(submit(pastedSubmit))
  expect(toasts).toEqual(['Operator answered Q17'])
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...userRow(pastedRow) })
    expect(await ui.find({ type: 'Text', text: /^✓ Operator answered Q17 — Long one\?$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^(a long answer ){79}a long answer$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /pasted_content/ })).toBeUndefined()
    await ui.unmount()
  }
})

test('asyncRewake wake: the "Stop hook feedback" row shows the answer it carried', async ($, on) => {
  engine(on)
  const { toasts, prompts } = sinks(on)
  await $.prompt.submit(submit(rewakeSubmit))
  expect(toasts).toEqual(['Operator answered Q15'])
  expect(prompts).toEqual([rewakeSubmit.text])  // the model's text is untouched
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...userRow(rewakeRow) })
    expect(await ui.find({ type: 'Text', text: /^✓ Operator answered Q15 — Merge now\?$/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^yes, after CI$/ })).toBeDefined()
    await ui.unmount()
    const expanded = await $.ui.mount({ plugin: P, surface, ...userRow(rewakeRow, true) })
    expect(await expanded.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
    await expanded.unmount()
  }
  // Another wake row (another plugin's, or one from before this load) has nothing queued: plain.
  const other = await $.ui.mount({ plugin: P, surface: 'terminal', ...userRow(rewakeRow), requestId: 'another-row' })
  expect(await other.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
  await other.unmount()
})

test('a non-inbox Stop hook wake keeps the pairing and its row', async ($, on) => {
  engine(on)
  const { toasts } = sinks(on)
  const foreign = rewakeSubmit.text.replace(/\[inbox\][^\n]*/, 'lint failed')
  await $.prompt.submit(submit({ text: foreign, origin: rewakeSubmit.origin }))
  await $.prompt.submit(submit(rewakeSubmit))
  expect(toasts).toEqual(['Operator answered Q15'])
  const first = await $.ui.mount({ plugin: P, surface: 'terminal', ...userRow(rewakeRow), requestId: 'wake-1' })
  expect(await first.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
  await first.unmount()
  const second = await $.ui.mount({ plugin: P, surface: 'terminal', ...userRow(rewakeRow), requestId: 'wake-2' })
  expect(await second.find({ type: 'Text', text: /^✓ Operator answered Q15/ })).toBeDefined()
  await second.unmount()
})

test('PostToolBatch additionalContext (drawn nowhere): a dim transcript line and a toast', async ($, on) => {
  const { toasts, logs } = sinks(on)
  on('classic.PostToolBatch', () => ({ ...postToolBatch.result, additionalContext: [...postToolBatch.result.additionalContext] }))
  const r = await $.classic.PostToolBatch({ tool_calls: [] } as never)
  expect(r).toEqual({ additionalContext: ['[inbox] Q12 answered — Which DB?: Postgres'] })  // the model's context, unchanged
  expect(logs).toEqual(['✓ Operator answered Q12 — Which DB?: Postgres'])
  expect(toasts).toEqual(['Operator answered Q12'])
})

test('Stop additionalContext (the engine draws it): a toast only', async ($, on) => {
  const { toasts, logs } = sinks(on)
  on('classic.Stop', () => ({ additionalContext: [...stop.result.additionalContext] }))
  const r = await $.classic.Stop({ stop_hook_active: false } as never)
  expect(r).toEqual({ additionalContext: ['[inbox] answered — Q13 Ship today?: yes · Q14 Which region?: eu-west-1'] })
  expect(logs).toEqual([])
  expect(toasts).toEqual(['Operator answered Q13, Q14'])
})

test('hook results without inbox answers stay quiet', async ($, on) => {
  const { toasts, logs } = sinks(on)
  on('classic.PostToolBatch', () => ({ additionalContext: ['lint: 3 warnings'] }))
  on('classic.Stop', () => ({}))
  await $.classic.PostToolBatch({ tool_calls: [] } as never)
  await $.classic.Stop({ stop_hook_active: false } as never)
  expect(logs).toEqual([])
  expect(toasts).toEqual([])
})
