import type { Register } from 'claude-code'

// herdr-inbox's MCP tools: `mcp__plugin_herdr-inbox_herdr-inbox__ask_operator` when installed
// as the plugin, `mcp__herdr-inbox__ask_operator` when the server is registered by hand.
const TOOL = /^mcp__(?:plugin_herdr-inbox_)?herdr-inbox__(\w+)$/
const SEVERITY: Record<string, string> = { info: 'cyan', warning: 'yellow', urgent: 'red' }
const DRAWN = new Set(['terminal', 'desktop'])

type Option = string | { label?: string; description?: string }
type Item = { question?: string; header?: string; options?: Option[]; multi_select?: boolean }
type Answer = { id: string; question: string; answer: string }

const str = (v: unknown) => (typeof v === 'string' ? v : '')
const label = (o: Option) => (typeof o === 'string' ? o : str(o?.label))

/** The `[inbox]` answers a message carries, in herdr-inbox's own delivery format
 *  (service.delivery_text): `[inbox] Q7 answered — q: a` or `[inbox] answered — Q7 q: a · Q8 q: b`.
 *  A big paste is recorded inside `<pasted_content …>`; the tags are dropped first. */
export function parseAnswers(raw: string): Answer[] {
  const text = raw.replace(/<\/?pasted_content[^>]*>/g, ' ')
  const split = (seg: string, id: string): Answer => {
    const at = seg.indexOf(': ')
    return at < 0 ? { id, question: '', answer: seg.trim() } : { id, question: seg.slice(0, at).trim(), answer: seg.slice(at + 2).trim() }
  }
  const single = /\[inbox\] (Q\d+) answered — ([\s\S]*)/.exec(text)
  if (single) return [split(single[2]!, single[1]!)]
  const batch = /\[inbox\] answered — ([\s\S]*)/.exec(text)
  if (!batch) return []
  return batch[1]!.split(/ · (?=Q\d+ )/).flatMap(seg => {
    const m = /^\s*(Q\d+) ([\s\S]*)$/.exec(seg)
    return m ? [split(m[2]!, m[1]!)] : []
  })
}

export const register: Register = on => {
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
    return <Text dimColor>✉ inbox: {compact[name]}</Text>
  })

  // The engine folds a run of MCP calls into one line ("Called herdr-inbox 3 times"); unfold a
  // run that asks or alerts so its cards show.
  on('ui.render', { component: 'ToolGroup' }, ($, e, next) => {
    const asks = e.props.calls.some(c => /^(ask|alert)_operator$/.test(TOOL.exec(c.tool)?.[1] ?? ''))
    if (!asks || e.props.isExpanded || !DRAWN.has(e.surface)) return next(e)
    return next({ ...e, props: { ...e.props, isExpanded: true } })
  })

  on('ui.render', { component: 'UserMessage' }, ($, e, next) => {
    if (e.props.isExpanded || !DRAWN.has(e.surface)) return next(e)
    const answers = parseAnswers(e.props.text)
    if (answers.length === 0) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box borderStyle="round" borderColor="green" paddingX={1} flexDirection="column">
        <Text bold color="green">✓ Answer from the operator</Text>
        {answers.map(a => (
          <Text key={a.id}><Text bold color="green">{a.id}</Text>{a.question ? <Text dimColor> {a.question}</Text> : ''}{'  →  '}{a.answer}</Text>
        ))}
      </Box>
    )
  })

  on('prompt.submit', ($, e, next) => {
    const ids = parseAnswers(e.text).map(a => a.id)
    if (ids.length > 0) $.ui.toast(`Operator answered ${ids.join(', ')}`)
    return next(e)
  })
}
