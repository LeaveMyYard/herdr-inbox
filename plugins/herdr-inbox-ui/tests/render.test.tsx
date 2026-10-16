import type { On } from 'claude-code'
import { test, expect } from 'claude-code/testing'

const P = 'herdr-inbox-ui'
const SURFACES = ['terminal', 'desktop'] as const
const tool = (name: string) => `mcp__plugin_herdr-inbox_herdr-inbox__${name}`
const row = (name: string, input: unknown, output?: unknown) => ({
  component: 'ToolUse' as const,
  props: { tool_use_id: 't1', tool: tool(name), input, output, isRunning: false, isErrored: false, isInterrupted: false },
})
const msg = (text: string, isExpanded = false) => ({
  component: 'UserMessage' as const,
  props: { text, origin: { kind: 'composer' as const }, isExpanded },
})

test('ask_operator card: title, questions, options, asked_by, filed ids', async $ => {
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...row('ask_operator', {
      title: 'Storage choice', asked_by: 'builder',
      questions: [
        { question: 'Which DB?', header: 'DB', options: ['Postgres', { label: 'SQLite', description: 'local' }] },
        { question: 'Ship today?', options: ['yes', 'no'], multi_select: true },
      ],
    }, { message: 'Filed Q12 (open), Q13 (open). Answers arrive later' }) })
    expect(await ui.find({ type: 'Text', text: /Question for the operator: Storage choice/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Which DB\?/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Postgres {2}· {2}SQLite/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /asked by builder/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /filed Q12 open, Q13 open/ })).toBeDefined()
    await ui.unmount()
  }
})

test('ask_operator card: single-question form', async $ => {
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...row('ask_operator', { question: 'Merge now?', options: ['yes', 'no'] }) })
    expect(await ui.find({ type: 'Text', text: /Merge now\?/ })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /yes {2}· {2}no/ })).toBeDefined()
    await ui.unmount()
  }
})

test('alert_operator card per severity', async $ => {
  for (const surface of SURFACES) {
    for (const severity of ['info', 'warning', 'urgent']) {
      const ui = await $.ui.mount({ plugin: P, surface, ...row('alert_operator', { title: 'Deploy stuck', message: 'needs eyes', severity, asked_by: 'ops' }) })
      expect(await ui.find({ type: 'Text', text: new RegExp(`Alert · ${severity}: Deploy stuck`) })).toBeDefined()
      expect(await ui.find({ type: 'Text', text: /needs eyes/ })).toBeDefined()
      await ui.unmount()
    }
  }
})

test('compact rows for check_answers and withdraw', async $ => {
  for (const surface of SURFACES) {
    for (const [name, input, text] of [
      ['check_answers', {}, /inbox: checked for operator answers/],
      ['withdraw_question', { id: 7, reason: 'solved it' }, /withdrew Q7 — solved it/],
      ['withdraw_alert', { id: 3 }, /withdrew alert A3/],
    ] as const) {
      const ui = await $.ui.mount({ plugin: P, surface, ...row(name, input) })
      expect(await ui.find({ type: 'Text', text })).toBeDefined()
      await ui.unmount()
    }
  }
})

// Stands for the engine's own drawing beneath the plugin: a row that says ENGINE.
const engineRow = (on: On) =>
  on('ui.render', ($, e) => { const { Text } = $.ui.resolve(e); return <Text>ENGINE</Text> })

test('other tools keep the engine row', async ($, on) => {
  engineRow(on)
  const ui = await $.ui.mount({ plugin: P, surface: 'terminal', component: 'ToolUse',
    props: { tool_use_id: 't2', tool: 'mcp__other__ask_operator', input: { question: 'x' }, isRunning: false, isErrored: false, isInterrupted: false } })
  expect(await ui.find({ type: 'Text', text: /Question for the operator/ })).toBeUndefined()
  expect(await ui.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
  await ui.unmount()
})

test('answer block: single, batch and paste-wrapped answers', async $ => {
  for (const surface of SURFACES) {
    const single = await $.ui.mount({ plugin: P, surface, ...msg('[inbox] Q12 answered — Which DB?: Postgres') })
    expect(await single.find({ type: 'Text', text: /Answer from the operator/ })).toBeDefined()
    expect(await single.find({ type: 'Text', text: /Q12 Which DB\? {2}→ {2}Postgres/ })).toBeDefined()
    await single.unmount()

    const batch = await $.ui.mount({ plugin: P, surface, ...msg('[inbox] answered — Q1 Which DB?: Postgres · Q2 Ship today?: yes, after CI') })
    expect(await batch.find({ type: 'Text', text: /Q1 Which DB\? {2}→ {2}Postgres$/ })).toBeDefined()
    expect(await batch.find({ type: 'Text', text: /Q2 Ship today\? {2}→ {2}yes, after CI/ })).toBeDefined()
    await batch.unmount()

    const pasted = await $.ui.mount({ plugin: P, surface,
      ...msg('\n\n<pasted_content id="1">\n[inbox] Q9 answered — Long one: a very long answer\n</pasted_content>') })
    expect(await pasted.find({ type: 'Text', text: /Q9 Long one {2}→ {2}a very long answer$/ })).toBeDefined()
    await pasted.unmount()
  }
})

test('expanded (ctrl+o), unrelated messages and other surfaces keep the raw row', async ($, on) => {
  engineRow(on)
  for (const surface of SURFACES) {
    const expanded = await $.ui.mount({ plugin: P, surface, ...msg('[inbox] Q12 answered — Which DB?: Postgres', true) })
    expect(await expanded.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
    await expanded.unmount()
    const plain = await $.ui.mount({ plugin: P, surface, ...msg('please mention [inbox] later') })
    expect(await plain.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
    await plain.unmount()
  }
  for (const surface of ['vscode', 'mobile'] as const) {
    const other = await $.ui.mount({ plugin: P, surface, ...msg('[inbox] Q12 answered — Which DB?: Postgres') })
    expect(await other.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
    await other.unmount()
  }
})

test('an arriving answer toasts and reaches the model unchanged', async ($, on) => {
  const toasts: string[] = []
  const seen: string[] = []
  on('ui.toast', (_$, e) => { toasts.push(e.text); return { value: undefined } })
  on('prompt.submit', (_$, e) => { seen.push(e.text); return { text: e.text } })
  const text = '[inbox] answered — Q1 a?: b · Q2 c?: d'
  await $.prompt.submit({ text, wait: false, origin: { kind: 'composer' } })
  await $.prompt.submit({ text: 'ordinary prompt', wait: false, origin: { kind: 'composer' } })
  expect(toasts).toEqual(['Operator answered Q1, Q2'])
  expect(seen).toEqual([text, 'ordinary prompt'])
})

test('a folded run of inbox calls unfolds when it asks or alerts', async ($, on) => {
  on('ui.render', ($, e) => {
    const { Text } = $.ui.resolve(e)
    return <Text>ENGINE expanded={String(e.component === 'ToolGroup' && e.props.isExpanded)}</Text>
  })
  const call = (name: string) => ({ tool: tool(name), input: {}, isRunning: false, isErrored: false, isInterrupted: false })
  for (const surface of SURFACES) {
    for (const [calls, expanded] of [[['check_answers', 'alert_operator'], 'true'], [['check_answers', 'list_my_questions'], 'false']] as const) {
      const ui = await $.ui.mount({ plugin: P, surface, component: 'ToolGroup',
        props: { calls: calls.map(call), isActive: false, isExpanded: false } })
      expect(await ui.find({ type: 'Text', text: `ENGINE expanded=${expanded}` })).toBeDefined()
      await ui.unmount()
    }
  }
})

test('a message that merely quotes an [inbox] line renders normally and does not toast', async ($, on) => {
  engineRow(on)
  const toasts: string[] = []
  on('ui.toast', (_$, e) => { toasts.push(e.text); return { value: undefined } })
  on('prompt.submit', (_$, e) => ({ text: e.text }))
  const quoted = 'why did you get "[inbox] Q12 answered — Which DB?: Postgres" twice?'
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: P, surface, ...msg(quoted) })
    expect(await ui.find({ type: 'Text', text: 'ENGINE' })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /Answer from the operator/ })).toBeUndefined()
    await ui.unmount()
  }
  await $.prompt.submit({ text: quoted, wait: false, origin: { kind: 'composer' } })
  expect(toasts).toEqual([])
})

test('a stray control character does not lose the card', async $ => {
  for (const surface of SURFACES) {
    const ask = await $.ui.mount({ plugin: P, surface, ...row('ask_operator', {
      question: '\u001b[31mWhich\u0000 DB?', options: ['Post\u0007gres'], asked_by: 'b\u0008uilder' }) })
    expect(await ask.find({ type: 'Text', text: /Question for the operator/ })).toBeDefined()
    expect(await ask.find({ type: 'Text', text: /^\[31mWhich DB\?$/ })).toBeDefined()
    expect(await ask.find({ type: 'Text', text: /Postgres/ })).toBeDefined()
    expect(await ask.find({ type: 'Text', text: /asked by builder/ })).toBeDefined()
    await ask.unmount()
    const answer = await $.ui.mount({ plugin: P, surface, ...msg('[inbox] Q4 answered — Which DB?: Post\u001bgres') })
    expect(await answer.find({ type: 'Text', text: /Q4 Which DB\? {2}→ {2}Postgres/ })).toBeDefined()
    await answer.unmount()
  }
})
