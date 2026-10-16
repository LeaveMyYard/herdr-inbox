import type { EngineInterface, Register } from 'claude-code'

// herdr-inbox's MCP tools: `mcp__plugin_herdr-inbox_herdr-inbox__ask_operator` when installed
// as the plugin, `mcp__herdr-inbox__ask_operator` when the server is registered by hand.
const TOOL = /^mcp__(?:plugin_herdr-inbox_)?herdr-inbox__(\w+)$/
const SEVERITY: Record<string, string> = { info: 'cyan', warning: 'yellow', urgent: 'red' }
const DRAWN = new Set(['terminal', 'desktop'])

type Option = string | { label?: string; description?: string }
type Item = { question?: string; header?: string; options?: Option[]; multi_select?: boolean }
type Answer = { id: string; question: string; answer: string }

// Control characters (an escape, a NUL) would get the tree refused and the card lost: drop them.
const clean = (s: string) => s.replace(/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/g, '')
const str = (v: unknown) => (typeof v === 'string' ? clean(v) : '')
const label = (o: Option) => str(typeof o === 'string' ? o : o?.label)

/** The `[inbox]` answers a message carries, in herdr-inbox's own delivery format
 *  (service.delivery_text): `[inbox] Q7 answered — q: a` or `[inbox] answered — Q7 q: a · Q8 q: b`.
 *  Read only at the start, like transcript.header_ids: a message that merely quotes an `[inbox]`
 *  line is not one. A big paste arrives as `<pasted_content id=…>\n[inbox] …\n</pasted_content id=…>`
 *  (the closing tag carries the id too, 2.1.301). */
export function parseAnswers(raw: string): Answer[] {
  if (!raw.includes('[inbox]')) return []
  const text = clean(raw.replace(/^\s*(?:<pasted_content[^>]*>\s*)?/, '').replace(/\s*<\/pasted_content[^>]*>\s*$/, ''))
  const split = (seg: string, id: string): Answer => {
    const at = seg.indexOf(': ')
    return at < 0 ? { id, question: '', answer: seg.trim() } : { id, question: seg.slice(0, at).trim(), answer: seg.slice(at + 2).trim() }
  }
  const single = /^\[inbox\] (Q\d+) answered — ([\s\S]*)/.exec(text)
  if (single) return [split(single[2]!, single[1]!)]
  const batch = /^\[inbox\] answered — ([\s\S]*)/.exec(text)
  if (!batch) return []
  return batch[1]!.split(/ · (?=Q\d+ )/).flatMap(seg => {
    const m = /^\s*(Q\d+) ([\s\S]*)$/.exec(seg)
    return m ? [split(m[2]!, m[1]!)] : []
  })
}

// asyncRewake (an idle session): Claude Code wakes it with a task notification whose summary is
// this and whose body is `Stop hook blocking error from command "…": [inbox] …` (2.1.301).
const REWAKE_SUMMARY = 'Stop hook feedback'
// The row arrives within a frame of its prompt. A row only takes a wake from this window, so a
// missed or late row never shifts a label onto another hook's feedback: it stays plain.
export const WAKE_WINDOW_MS = 5000
const wakes: { at: number; answers: Answer[] }[] = []
const wakeRows = new Map<string, Answer[]>()

export function rewakeAnswers(text: string): Answer[] {
  const m = /Stop hook blocking error from command "[^"]*": ([\s\S]*?)\s*<\/system-reminder>/.exec(text)
  return m ? parseAnswers(m[1]!) : []
}

export const register: Register = on => {
  // A new session (and each test) starts unpaired.
  on('session.start', ($, e, next) => {
    wakes.length = 0
    wakeRows.clear()
    return next(e)
  })

  on('ui.render', { component: 'ToolUse' }, ($, e, next) => {
    const name = TOOL.exec(e.props.tool)?.[1]
    if (!name || !DRAWN.has(e.surface)) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    const input = (e.props.input ?? {}) as Record<string, unknown>
    const by = str(input.asked_by) ? <Text dimColor>asked by {str(input.asked_by)}</Text> : null
    if (name === 'ask_operator') {
      const items: Item[] = Array.isArray(input.questions) ? (input.questions as Item[])
        : [{ question: str(input.question), header: str(input.header), options: input.options as Option[] }]
      const filed = [...JSON.stringify(e.props.output ?? '').matchAll(/Q(\d+) \((\w+)\)/g)].map(m => `Q${m[1]} ${m[2]}`)
      return (
        <Box borderStyle="round" borderColor="yellow" paddingX={1} flexDirection="column">
          <Text bold color="yellow">? Question for the operator{str(input.title) ? `: ${str(input.title)}` : ''}</Text>
          {items.map((q, i) => (
            <Box key={`q${i}`} flexDirection="column">
              <Text>{str(q.header) ? <Text bold>{str(q.header)} · </Text> : ''}{str(q.question)}</Text>
              {(q.options ?? []).length > 0 ? <Text dimColor>  {(q.options ?? []).map(label).join('  ·  ')}{q.multi_select ? '  (multi)' : ''}</Text> : null}
            </Box>
          ))}
          {by}
          {filed.length > 0 ? <Text color="green">filed {filed.join(', ')}</Text> : null}
        </Box>
      )
    }
    if (name === 'alert_operator') {
      const severity = str(input.severity) || 'warning'
      const color = SEVERITY[severity] ?? 'yellow'
      return (
        <Box borderStyle="round" borderColor={color} paddingX={1} flexDirection="column">
          <Text bold color={color}>⚑ Alert · {severity}: {str(input.title)}</Text>
          {str(input.message) ? <Text>{str(input.message)}</Text> : null}
          {by}
        </Box>
      )
    }
    const compact: Record<string, string> = {
      check_answers: 'checked for operator answers',
      list_my_questions: 'listed my questions',
      withdraw_question: `withdrew Q${String(input.id ?? '?')}${str(input.reason) ? ` — ${str(input.reason)}` : ''}`,
      withdraw_alert: `withdrew alert A${String(input.id ?? '?')}`,
    }
    if (!compact[name]) return next(e)
    return <Text dimColor>✉ inbox: {clean(compact[name])}</Text>
  })

  // The engine folds a run of MCP calls into one line ("Called herdr-inbox 3 times"); unfold a
  // run that asks or alerts so its cards show.
  on('ui.render', { component: 'ToolGroup' }, ($, e, next) => {
    const asks = e.props.calls.some(c => /^(ask|alert)_operator$/.test(TOOL.exec(c.tool)?.[1] ?? ''))
    if (!asks || e.props.isExpanded || !DRAWN.has(e.surface)) return next(e)
    return next({ ...e, props: { ...e.props, isExpanded: true } })
  })

  // A typed or pasted answer (UserMessage, origin composer), and an idle session's asyncRewake
  // wake: its row reads only "Stop hook feedback" (origin task-notification); the answer is in
  // the prompt prompt.submit saw just before: a row first drawn within WAKE_WINDOW_MS of a queued
  // wake takes the oldest such wake; any other row stays as Claude Code draws it.
  on('ui.render', { component: 'UserMessage' }, async ($, e, next) => {
    if (e.props.isExpanded || !DRAWN.has(e.surface)) return next(e)
    let answers = parseAnswers(e.props.text)
    if (answers.length === 0 && e.props.origin.kind === 'task-notification' && e.props.text === REWAKE_SUMMARY
        && e.requestId !== 'placeholder') {
      if (!wakeRows.has(e.requestId)) {
        const now = await $.clock.now()
        while (wakes.length > 0 && now - wakes[0]!.at > WAKE_WINDOW_MS) wakes.shift()
        wakeRows.set(e.requestId, wakes.shift()?.answers ?? [])
      }
      answers = wakeRows.get(e.requestId) ?? []
    }
    if (answers.length === 0) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box flexDirection="column">
        {answers.map(a => (
          <Box key={a.id} flexDirection="column">
            <Text><Text color="green">✓ Operator answered {a.id}</Text>{a.question ? <Text dimColor> — {a.question}</Text> : ''}</Text>
            <Box paddingLeft={2}><Text>{a.answer}</Text></Box>
          </Box>
        ))}
      </Box>
    )
  })

  on('prompt.submit', async ($, e, next) => {
    if (e.origin.kind === 'task-notification' && e.text.includes(`<summary>${REWAKE_SUMMARY}</summary>`)) {
      const answers = rewakeAnswers(e.text)
      wakes.push({ at: await $.clock.now(), answers })  // every wake, inbox or not: rows pair in order
      announce($, answers)
    } else {
      announce($, parseAnswers(e.text))
    }
    return next(e)
  })

  // A busy session gets answers as hook additionalContext. PostToolBatch's is drawn nowhere: a
  // dim transcript line (ui.log, never sent to the model) shows it. Stop's is drawn by the
  // engine itself ("Stop hook feedback: [inbox] …", no render site), so only the toast.
  on('classic.PostToolBatch', async ($, e, next) => {
    const r = await next(e)
    const answers = (r.additionalContext ?? []).flatMap(parseAnswers)
    for (const a of answers) $.ui.log(`✓ Operator answered ${a.id}${a.question ? ` — ${a.question}` : ''}: ${a.answer}`)
    announce($, answers)
    return r
  })

  on('classic.Stop', async ($, e, next) => {
    const r = await next(e)
    announce($, (r.additionalContext ?? []).flatMap(parseAnswers))
    return r
  })
}

function announce($: EngineInterface, answers: Answer[]) {
  if (answers.length > 0) $.ui.toast(`Operator answered ${answers.map(a => a.id).join(', ')}`)
}
