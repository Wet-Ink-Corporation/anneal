// The cockpit's band, pane, refresh, `/anneal` and its calibration dashboard. `claude plugin
// test plugin` runs it; the test's own hooks stand for the host (`$.process.run`, the clock,
// the surfaces), so nothing runs and no model is called. Fixtures are `anneal status --json`
// and `anneal calibrate` envelopes as the CLI prints them (docs/status.md, docs/calibrate.md).

import { expect, mock, test } from 'claude-code/testing'
import type { CommandRunInput, Hook, On, ProcessRunResult, StateRead, ValueOrDeny } from 'claude-code'
import type { Engine, MockClock } from 'claude-code/testing'

/** The surfaces the band and pane are drawn on, one loop over both. */
const SURFACES = ['terminal', 'desktop'] as const

const FINDINGS_NONE = '"findings":[]'
const PROVES = '"proves":"the ledger was read","does_not_prove":"that the work was done"'

/** No ledger yet: `anneal status` exits 2 with its `status/no-ledger` finding. */
const EMPTY = {
  exitCode: 2,
  stdout:
    '{"ok":false,"command":"status","data":{"schema":1},"findings":[{"key":"status/no-ledger",' +
    `"message":"no ledger exists yet"}],${PROVES}}`,
}

/** Mid-build: an approved plan, 3 of 7 tasks done, an old block since passed, 58% context. */
const MID_BUILD_DATA = {
  schema: 1,
  phase: { mode: 'build', playbook: { name: 'build', requires: ['test_run'] }, event: 1 },
  plan: {
    id: 'docs/plan.md',
    status: 'approved',
    hash: 'sha256:a1b2c3d4e5f6',
    approved: { hash: 'sha256:a1b2c3d4e5f6', event: 2, ts: '2026-10-19T10:00:02Z' },
  },
  tasks: { done: 3, total: 7, next: { id: 'T4', title: 'CLI flag' } },
  gates: [
    {
      id: 'preconditions.plan',
      kind: 'gate_blocked',
      reason: 'the plan is not approved',
      event: 3,
      ts: '2026-10-19T10:00:03Z',
    },
  ],
  verdict: { pr: 7, outcome: 'no_verdict', valid: false, stale: 0 },
  clef: {
    configured: true,
    points: [{ point: 'D1', answered: 3, configured_mode: 'enforce', mode: 'shadow' }],
  },
  context: { used_tokens: 116800, window: 200000, percent: 58.4, level: null },
  ledger_tail: [
    { event: 3, type: 'gate_blocked', summary: 'gate_blocked point:preconditions.plan', ts: '2026-10-19T10:00:03Z' },
    { event: 4, type: 'task_done', summary: 'task_done task:T3', ts: '2026-10-19T10:00:04Z' },
  ],
}
const MID_BUILD = { exitCode: 0, stdout: envelope(MID_BUILD_DATA) }

/** Blocked: a gate blocked after the newest progress, so it is still open. */
const BLOCKED_DATA = {
  ...MID_BUILD_DATA,
  gates: [
    ...MID_BUILD_DATA.gates,
    {
      id: 'guard.rm',
      kind: 'gate_blocked',
      reason: 'rm -rf outside the worktree',
      event: 5,
      ts: '2026-10-19T10:00:05Z',
    },
  ],
  ledger_tail: [
    ...MID_BUILD_DATA.ledger_tail,
    { event: 5, type: 'gate_blocked', summary: 'gate_blocked point:guard.rm', ts: '2026-10-19T10:00:05Z' },
  ],
}
const BLOCKED = { exitCode: 0, stdout: envelope(BLOCKED_DATA) }

function envelope(data: unknown): string {
  return `{"ok":true,"command":"status","data":${JSON.stringify(data)},${FINDINGS_NONE},${PROVES}}\n`
}

/** What a child that exited with `exitCode` wrote. */
const ran = (exitCode: number, stdout: string, stderr = ''): ProcessRunResult => ({
  exitCode,
  stdout,
  stderr,
  isStdoutTruncated: false,
  isStderrTruncated: false,
})

/** `/anneal` typed at the prompt, as the engine stamps it. */
const ANNEAL: CommandRunInput = {
  command: 'anneal',
  args: '',
  origin: { kind: 'composer' },
  presentation: { isFullscreen: true, columns: 160 },
}

const SCROLL = { offset: 0, bodyRows: 10 }
const BAND_PROPS = {
  hasSurvey: false,
  isWorking: false,
  maxRows: 6,
  bodyColumns: 120,
  scroll: SCROLL,
  view: {},
}
const PANE_PROPS = {
  title: 'anneal',
  isFocused: false,
  bodyColumns: 80,
  placement: 'dock' as const,
  scroll: { offset: 0, bodyRows: 40 },
  view: {},
}

type Answer = { exitCode: number; stdout: string; stderr?: string } | 'timeout'

/** The host beneath the plugin: the clock, the surfaces, the pane list, the CLI and the log. */
type Host = {
  clock: MockClock
  runs: (readonly string[])[]
  timeouts: (number | undefined)[]
  logged: string[]
  opened: string[]
  answer: { next: Answer }
  /** `anneal calibrate`'s runs, answered apart from `anneal status`'s. */
  calibrateRuns: (readonly string[])[]
  calibrateTimeouts: (number | undefined)[]
  calibrate: { next: Answer }
  /** The launcher (`anneal`, `anneal.cmd`) of every `process.run`, in order, started or not. */
  launched: string[]
  /** Launchers that cannot start (a spawn error) or that start and time out; the rest run. */
  launchers: Record<string, 'ENOENT' | 'EACCES' | 'timeout'>
  /** Runs wait for these before answering, with the answer chosen when they were called. */
  hold: { status?: Promise<void>; calibrate?: Promise<void> }
  /** Host calls refused while set. */
  refuse: { panes?: string }
}

/** What a test answers for `$.state.get` of a key in place of the store; absent passes through. */
type StateAnswer = (key: string) => ValueOrDeny<StateRead<unknown>> | undefined

/**
 * `on` with `mock.clock`'s own `clock.now` hook behind a refusal: while `refusals.now` is above 0
 * each read is refused and counts it down, after which the mock clock answers. An event takes one
 * hook, so the refusal wraps the mock's rather than sitting beside it.
 */
const refusingNow = (on: On, refusals: { now: number }): On => {
  const wrapped = (event: string, ...rest: unknown[]): unknown => {
    const hook = rest[0]
    if (event !== 'clock.now' || rest.length !== 1 || typeof hook !== 'function') {
      return (on as (event: string, ...rest: unknown[]) => unknown)(event, ...rest)
    }
    const answer = hook as Hook<'clock.now'>
    return on('clock.now', async ($, e, next) => {
      if (refusals.now <= 0) return answer($, e, next)
      refusals.now -= 1
      return { deny: 'the clock is unavailable' }
    })
  }
  return wrapped as On
}

/** Each test starts its clock far from the last, as the module's pacing outlives a test. */
let epoch = 0

const host = (
  on: On,
  first: Answer,
  options: {
    surfaces?: readonly string[]
    calibrate?: Answer
    /** `$.clock.now` refused while `now` is above 0, each refusal counting it down. */
    refuseClock?: { now: number }
    /** Stands in for the store's answers to `$.state.get`. */
    state?: StateAnswer
  } = {},
): Host => {
  epoch += 1_000_000
  const clock = mock.clock(options.refuseClock === undefined ? on : refusingNow(on, options.refuseClock), {
    now: epoch,
  })
  const state: Host = {
    clock,
    runs: [],
    timeouts: [],
    logged: [],
    opened: [],
    answer: { next: first },
    calibrateRuns: [],
    calibrateTimeouts: [],
    calibrate: { next: options.calibrate ?? { exitCode: 2, stdout: '', stderr: 'ERROR no calibrate answer\n' } },
    launched: [],
    launchers: {},
    hold: {},
    refuse: {},
  }
  const answerState = options.state
  if (answerState !== undefined) {
    on('state.get', async (_$, e, next) => answerState(e.key) ?? next(e))
  }
  on('session.surfaces', async () => ({ value: (options.surfaces ?? ['terminal']) as never }))
  on('ui.panes', async () =>
    state.refuse.panes !== undefined ? { deny: state.refuse.panes } : {
    value: state.opened.map(id => ({ id, title: id, isShown: true, isFocused: false, isPlaced: true })),
  })
  on('ui.open', async (_$, e) => {
    state.opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.close', async () => ({ value: undefined }))
  on('command.register', async (_$, e) => ({ value: { command: e.name } }))
  on('ui.log', async (_$, e) => {
    state.logged.push(String(e.text))
    return { value: undefined }
  })
  on('tool.call', async () => ({ result: 'ok' }))
  on('turn.complete', async (_$, e) => ({ text: e.answer }))
  on('session.start', async (_$, e) => ({ cwd: e.cwd }))
  // The engine's own band, which the cockpit passes to while it has nothing to show.
  on('ui.render', { component: 'AbovePrompt' }, async ($, e) => {
    const { Box } = $.ui.resolve(e)
    return Box({ key: 'engine' })
  })
  on('process.run', async (_$, e) => {
    const launcher = String(e.argv[0]).split('/').pop() ?? ''
    state.launched.push(launcher)
    const fault = state.launchers[launcher]
    if (fault === 'timeout') return { deny: `timed out after ${e.init?.timeoutMs} ms` }
    if (fault !== undefined) return { deny: `spawn ${launcher} ${fault}` }
    if (e.argv[1] === 'calibrate') {
      state.calibrateRuns.push(e.argv)
      state.calibrateTimeouts.push(e.init?.timeoutMs)
      const answer = state.calibrate.next
      await state.hold.calibrate
      if (answer === 'timeout') return { deny: 'timed out after 10000 ms' }

      return { value: ran(answer.exitCode, answer.stdout, answer.stderr) }
    }
    state.runs.push(e.argv)
    state.timeouts.push(e.init?.timeoutMs)
    const answer = state.answer.next
    await state.hold.status
    if (answer === 'timeout') return { deny: 'timed out after 2000 ms' }

    return { value: ran(answer.exitCode, answer.stdout, answer.stderr) }
  })

  return state
}

/** A turn ending, as the engine raises it. */
const turnComplete = ($: Engine) =>
  $.turn.complete({ answer: 'done', durationMs: 10, isAborted: false, turnId: 't1', reason: 'answer' })

/** Ends a turn and lets the refresh it asks for run. */
const refreshed = async ($: Engine, state: Host): Promise<void> => {
  await state.clock.advance(1_000)
  await turnComplete($)
  await state.clock.settle()
}

const band = async ($: Engine, surface: (typeof SURFACES)[number]) =>
  $.ui.mount({ plugin: 'anneal', surface, component: 'AbovePrompt', props: BAND_PROPS })

const pane = async ($: Engine, surface: (typeof SURFACES)[number]) =>
  $.ui.mount({ plugin: 'anneal', surface, component: 'Pane', requestId: 'anneal', props: PANE_PROPS })

test('the band and pane draw nothing of anneal while there is no ledger', async ($, on) => {
  const state = host(on, EMPTY)
  await refreshed($, state)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect(await ui.find({ key: 'band' })).toBe(undefined)
    expect(await ui.find({ key: 'engine' })).not.toBe(undefined)
    await ui.unmount()
    const opened = await pane($, surface)
    expect((await opened.find({ key: 'note' }))?.text).toBe('No ledger here yet: nothing has been recorded.')
    await opened.unmount()
  }
})

test('mid-build: the band is one dim row of mode, plan, tasks and context', async ($, on) => {
  const state = host(on, MID_BUILD)
  await refreshed($, state)
  expect(state.runs.length).toBe(1)
  expect(state.runs[0]?.slice(1)).toEqual([
    'status',
    '--json',
    '--fields',
    'phase,plan,tasks,gates,context,ledger_tail',
    '--tail',
    '10',
  ])
  expect(state.timeouts[0]).toBe(2_000)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    const row = await ui.find({ key: 'band' })
    expect(row?.text).toBe('anneal · build · plan ✓ a1b2c3 · task 3/7 · ctx 58%')
    const text = await ui.find({ type: 'Text' })
    expect(text?.props.color).toBe(undefined)
    expect(text?.props.dimColor).toBe(true)
    await ui.unmount()
  }
})

test('the band is cut to the columns it is given', async ($, on) => {
  const state = host(on, MID_BUILD)
  await refreshed($, state)
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({
      plugin: 'anneal',
      surface,
      component: 'AbovePrompt',
      props: { ...BAND_PROPS, bodyColumns: 20 },
    })
    expect((await ui.find({ key: 'band' }))?.text).toBe('anneal · build · pl…')
    await ui.unmount()
  }
})

test('a blocked gate turns the band to the warning color, with its reason', async ($, on) => {
  const state = host(on, BLOCKED)
  await refreshed($, state)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    const row = await ui.find({ key: 'band' })
    expect(row?.text).toBe(
      'anneal · build · plan ✓ a1b2c3 · task 3/7 · 1 gate open · ctx 58% · guard.rm: rm -rf outside the worktree',
    )
    expect((await ui.find({ type: 'Text' }))?.props.color).toBe('warning')
    await ui.unmount()
  }
})

test('/anneal opens the pane, which then asks for its sections too', async ($, on) => {
  const state = host(on, BLOCKED)
  const answer = await $.command.run(ANNEAL)
  expect(answer.text).toBe('anneal: cockpit pane opened.')
  expect(state.opened).toEqual(['anneal'])
  await state.clock.settle()
  expect(state.runs[0]?.[4]).toBe('phase,plan,tasks,gates,context,ledger_tail,verdict,clef')
  for (const surface of SURFACES) {
    const ui = await pane($, surface)
    for (const heading of ['Gates', 'Plan', 'Tasks', 'Verdict for HEAD', 'Clef', 'Ledger']) {
      expect(await ui.find({ type: 'Text', text: heading })).not.toBe(undefined)
    }
    expect((await ui.find({ key: 'plan' }))?.text).toBe(
      'docs/plan.md: approved a1b2c3, approved 2026-10-19T10:00:02Z',
    )
    expect((await ui.find({ key: 'tasks' }))?.text).toBe('3/7 done, next T4 CLI flag')
    expect((await ui.find({ key: 'verdict' }))?.text).toBe('no_verdict (PR #7)')
    expect((await ui.find({ key: 'clef:D1' }))?.text).toBe('D1 shadow, 3 answered (configured enforce)')
    expect((await ui.find({ key: 'ledger:5' }))?.text).toBe(
      '#5 gate_blocked point:guard.rm (2026-10-19T10:00:05Z)',
    )
    expect((await ui.find({ key: 'gate:5' }))?.text).toBe('▸ #5 guard.rm open')
    expect((await ui.find({ key: 'gate:3' }))?.text).toBe('▸ #3 preconditions.plan passed')
    expect(await ui.find({ key: 'gate-reason:5' })).toBe(undefined)
    await ui.press({ key: 'gate:5' })
    expect((await ui.find({ key: 'gate-reason:5' }))?.text).toBe('  rm -rf outside the worktree')
    expect((await ui.find({ key: 'gate-kind:5' }))?.text).toBe('  gate_blocked at 2026-10-19T10:00:05Z')
    await ui.press({ key: 'gate:5' })
    expect(await ui.find({ key: 'gate-reason:5' })).toBe(undefined)
    await ui.unmount()
  }
})

test('the pane never opens unasked', async ($, on) => {
  const state = host(on, BLOCKED)
  await $.session.start({ cwd: '/work', surface: 'terminal', isInteractive: true })
  await state.clock.settle()
  await refreshed($, state)
  await $.tool.call({ tool: 'Bash', command: 'ls' })
  await state.clock.advance(1_000)
  expect(state.opened).toEqual([])
  expect(state.runs.length).toBe(3)
})

test('where nothing draws, /anneal answers with the status text form', async ($, on) => {
  const state = host(on, { exitCode: 0, stdout: 'phase: build\nplan: approved\n' }, { surfaces: [] })
  const answer = await $.command.run(ANNEAL)
  expect(answer.text).toBe('phase: build\nplan: approved')
  expect(state.opened).toEqual([])
  expect(state.runs.length).toBe(1)
  expect(state.runs[0]?.slice(1)).toEqual(['status'])
})

test('where nothing draws and the CLI has no status, /anneal says so in one line', async ($, on) => {
  host(on, { exitCode: 2, stdout: '', stderr: "ERROR unrecognized subcommand 'status'\n\nUsage: anneal\n" }, {
    surfaces: [],
  })
  const answer = await $.command.run(ANNEAL)
  expect(answer.text).toBe("anneal status is not built yet (ERROR unrecognized subcommand 'status').")
})

test('ten tool calls in one second cause at most two CLI runs', async ($, on) => {
  const state = host(on, MID_BUILD)
  for (let call = 0; call < 10; call += 1) {
    await $.tool.call({ tool: call % 2 === 0 ? 'Bash' : 'Edit', command: 'ls' } as never)
    await state.clock.advance(100)
  }
  expect(state.runs.length).toBeLessThanOrEqual(2)
  await state.clock.advance(2_000)
  expect(state.runs.length).toBe(2)
})

test('a tool the status does not follow asks for no refresh', async ($, on) => {
  const state = host(on, MID_BUILD)
  await $.tool.call({ tool: 'Read', file_path: '/work/README.md' } as never)
  await state.clock.advance(2_000)
  expect(state.runs.length).toBe(0)
})

test('a CLI timeout keeps the last good state on screen and logs nothing', async ($, on) => {
  const state = host(on, BLOCKED)
  const session = mock.session(on)
  await refreshed($, state)
  state.answer.next = 'timeout'
  await refreshed($, state)
  expect(state.runs.length).toBe(2)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect((await ui.find({ type: 'Text' }))?.props.color).toBe('warning')
    await ui.unmount()
  }
  expect(state.logged).toEqual([])
  expect(session.appended().length).toBe(0)
})

test('a CLI that fails shows status unavailable once, and the next refresh retries', async ($, on) => {
  const state = host(on, { exitCode: 127, stdout: '', stderr: 'anneal: no anneal binary\n' })
  await refreshed($, state)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect((await ui.find({ key: 'band' }))?.text).toBe('anneal · status unavailable')
    await ui.unmount()
  }
  state.answer.next = MID_BUILD
  await refreshed($, state)
  expect(state.runs.length).toBe(2)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect((await ui.find({ key: 'band' }))?.text).toBe('anneal · build · plan ✓ a1b2c3 · task 3/7 · ctx 58%')
    await ui.unmount()
  }
  expect(state.logged).toEqual([])
})

// ---- Calibration (#63) --------------------------------------------------------------------------
//
// Fixtures are `anneal calibrate` envelopes in the shape docs/calibrate.md documents (Output,
// "The `data` schema"), with each point's real thresholds from plugin/playbooks/clef-thresholds.json.
// D1's partial numbers are docs/calibrate.md's worked example as the CLI prints it.

/** `/anneal calibration` typed at the prompt of a wide fullscreen terminal. */
const CALIBRATION: CommandRunInput = { ...ANNEAL, args: 'calibration' }
const CALIBRATION_REFRESH: CommandRunInput = { ...ANNEAL, args: 'calibration refresh' }

const CALIBRATION_PANE_PROPS = { ...PANE_PROPS, title: 'calibration' }

const APPLY_NOTE = 'Proposals only, nothing applied. A person promotes with'
const APPLY_COMMAND = '  anneal calibrate --apply --thresholds <file>'

/** The diagram's geometry, as cockpit.tsx draws it: the raster's plot rows and the SVG's square. */
const PLOT_ROWS = 11
const SVG_LEFT = 36
const SVG_TOP = 10
const SVG_PLOT = 200

type BinSpec = Readonly<Record<number, readonly [count: number, mean: number, observed: number]>>

/** Ten bins, empty but for `filled`: bin index to count, mean probability, observed frequency. */
const bins = (filled: BinSpec = {}) =>
  Array.from({ length: 10 }, (_, index) => {
    const bin = filled[index]
    return {
      lower: index / 10,
      upper: (index + 1) / 10,
      count: bin?.[0] ?? 0,
      mean_probability: bin?.[1] ?? null,
      observed_frequency: bin?.[2] ?? null,
    }
  })

/** Each point's questions and lowest band, from the shipped thresholds. */
const RULES = [
  ['D1', 'noul', 'destructive_or_irreversible', 'ask', '/points/D1/rules/noul/bands/1/at_least', 0.4, 0.9, 'below_threshold'],
  ['D2', 'noul', 'one_way_door', 'park', '/points/D2/rules/noul/bands/0/at_least', 0.5, null, 'below_threshold'],
  ['D3', 'choice', 'phase', 'hint', '/points/D3/rules/choice/bands/0/at_least', 0.6, null, 'at_or_above_threshold'],
  ['D3', 'score', 'size', null, null, null, null, 'at_or_above_threshold'],
  ['D4', 'noul', 'present_and_grounded', 'pass', '/points/D4/rules/noul/bands/0/at_least', 0.5, null, 'at_or_above_threshold'],
  ['D5', 'score', 'defect_risk', 'doubt_review', '/points/D5/rules/score/bands/0/at_least', 0.6666, null, 'below_threshold'],
  ['D6', 'choice', 'ruled_before', 'surface', '/points/D6/rules/choice/bands/0/at_least', 0.7, null, 'at_or_above_threshold'],
  ['D7', 'noul', 'is_decision', 'ask_human', '/points/D7/rules/noul/bands/0/at_least', 0.5, null, 'below_threshold'],
] as const

type QuestionData = Record<string, unknown> & { readonly verdict: string }

/** A question with no decisions, as calibrate reports it. */
const emptyQuestion = (rule: (typeof RULES)[number]): QuestionData => {
  const [, question, subject, band, pointer, current, ceiling, falseAccept] = rule
  return {
    question,
    subject,
    n: 0,
    labelled: 0,
    unanswered: 0,
    unusable_labels: 0,
    brier: null,
    bins: bins(),
    asks: 0,
    ask_rate: null,
    threshold: {
      band,
      pointer,
      current,
      ceiling,
      false_accept: falseAccept,
      current_expected_cost: null,
      proposed: null,
      proposed_expected_cost: null,
      no_proposal: band === null ? 'no_bands' : 'no_labels',
    },
    verdict: 'stay-shadow',
    reasons: ['too_few_labelled'],
  }
}

/** Every point D1 to D7, each question replaced where `with` names it (`D1:noul`). */
const points = (replaced: Readonly<Record<string, (base: QuestionData) => QuestionData>> = {}) =>
  ['D1', 'D2', 'D3', 'D4', 'D5', 'D6', 'D7'].map(point => {
    const questions = RULES.filter(rule => rule[0] === point).map(rule => {
      const base = emptyQuestion(rule)
      const change = replaced[`${point}:${rule[1]}`]
      return change === undefined ? base : change(base)
    })
    const counted = questions.filter(question => (question.threshold as { band: unknown }).band !== null)
    const isEligible = counted.length > 0 && counted.every(question => question.verdict === 'eligible-to-enforce')
    return { point, verdict: isEligible ? 'eligible-to-enforce' : 'stay-shadow', questions }
  })

const CALIBRATE_PROVES =
  '"proves":"the calibration run completed and was recorded",' +
  '"does_not_prove":"that the decision point is calibrated well enough to enforce"'

function calibrateEnvelope(data: readonly unknown[]): { exitCode: number; stdout: string } {
  const body = {
    log: '/home/u/.anneal/decisions/decisions.jsonl',
    files: ['/home/u/.anneal/decisions/decisions.jsonl'],
    proposals: '/home/u/.anneal/decisions/proposals.json',
    thresholds: '/plugin/playbooks/clef-thresholds.json',
    costs: { false_accept: 5, needless: 1 },
    skipped_rows: 0,
    unmatched_labels: 0,
    unreported_rows: 0,
    points: data,
    apply: [],
    diff: [],
  }
  return {
    exitCode: 0,
    stdout: `{"ok":true,"command":"calibrate","data":${JSON.stringify(body)},${FINDINGS_NONE},${CALIBRATE_PROVES}}\n`,
  }
}

/** docs/calibrate.md's worked example: D1 with five labelled decisions of six. */
const WORKED_D1 = (base: QuestionData): QuestionData => ({
  ...base,
  n: 6,
  labelled: 5,
  brier: 0.2885,
  bins: bins({ 1: [1, 0.1, 0], 3: [1, 0.3, 1], 8: [1, 0.8, 1], 9: [2, 0.975, 0.5] }),
  asks: 2,
  ask_rate: 0.3333333333333333,
  threshold: {
    ...(base.threshold as object),
    current_expected_cost: 1.2,
    proposed: 0.3,
    proposed_expected_cost: 0.2,
    no_proposal: null,
  },
  reasons: ['too_few_labelled', 'brier_above_bound', 'ask_rate_at_or_above_bound'],
})

/** D5 ready: 60 labelled, Brier 0.08, no asks, one bin under five decisions. */
const READY_D5_BINS: BinSpec = {
  1: [12, 0.12, 0.083],
  3: [9, 0.34, 0.333],
  5: [3, 0.55, 0.667],
  7: [14, 0.74, 0.714],
  9: [22, 0.93, 0.92],
}
const READY_D5 = (base: QuestionData): QuestionData => ({
  ...base,
  n: 64,
  labelled: 60,
  brier: 0.0812,
  bins: bins(READY_D5_BINS),
  asks: 0,
  ask_rate: 0,
  threshold: {
    ...(base.threshold as object),
    current_expected_cost: 0.15,
    proposed: 0.6666,
    proposed_expected_cost: 0.15,
    no_proposal: null,
  },
  verdict: 'eligible-to-enforce',
  reasons: [],
})

/** D7 with labels enough and a Brier score over the bound. */
const BRIER_D7 = (base: QuestionData): QuestionData => ({
  ...base,
  n: 58,
  labelled: 55,
  brier: 0.21,
  bins: bins({ 2: [20, 0.25, 0.4], 6: [35, 0.65, 0.3] }),
  asks: 6,
  ask_rate: 0.1034,
  threshold: {
    ...(base.threshold as object),
    current_expected_cost: 0.9,
    proposed: 0.5,
    proposed_expected_cost: 0.9,
    no_proposal: null,
  },
  reasons: ['brier_above_bound'],
})

const CAL_EMPTY = calibrateEnvelope(points())
const CAL_PARTIAL = calibrateEnvelope(points({ 'D1:noul': WORKED_D1 }))
const CAL_READY = calibrateEnvelope(points({ 'D1:noul': WORKED_D1, 'D5:score': READY_D5, 'D7:noul': BRIER_D7 }))

/** Mid-build with D5 configured and running as `enforce`. */
const CAL_STATUS = {
  exitCode: 0,
  stdout: envelope({
    ...MID_BUILD_DATA,
    clef: {
      configured: true,
      points: [
        { point: 'D1', answered: 6, configured_mode: 'enforce', mode: 'shadow' },
        { point: 'D5', answered: 64, mode: 'enforce' },
      ],
    },
  }),
}

const calibrationPane = async ($: Engine, surface: (typeof SURFACES)[number]) =>
  $.ui.mount({
    plugin: 'anneal',
    surface,
    component: 'Pane',
    requestId: 'anneal-calibration',
    props: CALIBRATION_PANE_PROPS,
  })

/** Opens the calibration pane and lets its first `anneal calibrate` and status refresh run. */
const openCalibration = async ($: Engine, state: Host): Promise<void> => {
  expect((await $.command.run(CALIBRATION)).text).toBe('anneal: calibration pane opened.')
  await state.clock.settle()
}

/** Standard base64 back to bytes. */
const fromBase64 = (text: string): number[] => {
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
  const bytes: number[] = []
  for (let index = 0; index < text.length; index += 4) {
    const sextets = [0, 1, 2, 3].map(offset => alphabet.indexOf(text.charAt(index + offset)))
    const word = sextets.reduce((all, sextet) => (all << 6) | Math.max(0, sextet), 0)
    bytes.push((word >> 16) & 0xff)
    if (text.charAt(index + 2) !== '=') bytes.push((word >> 8) & 0xff)
    if (text.charAt(index + 3) !== '=') bytes.push(word & 0xff)
  }
  return bytes
}

/** A Raster's cells read back: each cell's glyph, column and row. */
const rasterCells = (props: Record<string, unknown>) => {
  const columns = Number(props.columns)
  const bytes = fromBase64(String(props.cells))
  const cells: { column: number; row: number; glyph: string }[] = []
  for (let index = 0; index * 12 < bytes.length; index += 1) {
    const at = index * 12
    const code = (bytes[at] ?? 0) | ((bytes[at + 1] ?? 0) << 8) | ((bytes[at + 2] ?? 0) << 16) | ((bytes[at + 3] ?? 0) << 24)
    cells.push({ column: index % columns, row: Math.floor(index / columns), glyph: String.fromCodePoint(code) })
  }
  return cells
}

/** What each tree says of the bins it draws: which bin, filled or hollow, observed in tenths. */
type DrawnBin = { bin: number; isHollow: boolean; observedTenths: number; isUpperHalf: boolean }

const binsOfRaster = (props: Record<string, unknown>): DrawnBin[] =>
  rasterCells(props)
    .filter(cell => cell.glyph === '●' || cell.glyph === '○')
    .map(cell => ({
      bin: Math.floor((cell.column - 1) / 2),
      isHollow: cell.glyph === '○',
      observedTenths: PLOT_ROWS - 1 - cell.row,
      isUpperHalf: (cell.column - 1) % 2 === 1,
    }))
    .sort((a, b) => a.bin - b.bin)

const binsOfSvg = (source: string): DrawnBin[] =>
  [...source.matchAll(/<circle class="bin (filled|hollow)" data-bin="(\d+)" data-count="\d+" cx="([\d.]+)" cy="([\d.]+)"/g)]
    .map(match => {
      const bin = Number(match[2])
      const predicted = (Number(match[3]) - SVG_LEFT) / SVG_PLOT
      const observed = (SVG_TOP + SVG_PLOT - Number(match[4])) / SVG_PLOT
      return {
        bin,
        isHollow: match[1] === 'hollow',
        observedTenths: Math.round(observed * 10),
        isUpperHalf: predicted * 10 - bin >= 0.5,
      }
    })
    .sort((a, b) => a.bin - b.bin)

/** Each point's two lines with no decisions: D1's mode is in the status fixture, the rest are not. */
const ROWS_EMPTY: Record<string, readonly [string, string]> = {
  D1: ['▸ D1 shadow ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.4'],
  D2: ['▸ D2 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.5'],
  D3: ['▸ D3 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.6'],
  D4: ['▸ D4 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.5'],
  D5: ['▸ D5 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.6666'],
  D6: ['▸ D6 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.7'],
  D7: ['▸ D7 unknown ░░░░░░░░░░ 0/50 · short of labels', '   Brier – · ask – · threshold 0.5'],
}

test('calibration with no decisions: a row per point, each short of labels, and no apply button', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_EMPTY })
  await openCalibration($, state)
  expect(state.opened).toEqual(['anneal-calibration'])
  expect(state.calibrateRuns.map(argv => argv.slice(1))).toEqual([['calibrate']])
  expect(state.calibrateTimeouts).toEqual([10_000])
  // Only the calibration pane is open, so the status refresh adds `clef` for each point's mode.
  expect(state.runs[0]?.[4]).toBe('phase,plan,tasks,gates,context,ledger_tail,clef')
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect(await ui.find({ type: 'Text', text: 'Calibration' })).not.toBe(undefined)
    expect((await ui.find({ key: 'cal-empty' }))?.text).toBe('No decisions logged yet: every point is short of labels.')
    for (const [point, [head, stats]] of Object.entries(ROWS_EMPTY)) {
      expect((await ui.find({ key: `cal:${point}` }))?.text).toBe(head)
      expect((await ui.find({ key: `cal-stats:${point}` }))?.text).toBe(stats)
    }
    expect((await ui.find({ key: 'cal-apply' }))?.text).toBe(APPLY_NOTE)
    expect((await ui.find({ key: 'cal-apply-command' }))?.text).toBe(APPLY_COMMAND)
    expect(await ui.find({ type: 'Button', text: /apply/i })).toBe(undefined)
    await ui.press({ key: 'cal:D3' })
    expect((await ui.find({ key: 'cal-threshold:D3:choice' }))?.text).toBe('  choice: hint at ≥ 0.6, no proposal (no_labels)')
    expect((await ui.find({ key: 'cal-threshold:D3:score' }))?.text).toBe('  score: no threshold (no_bands)')
    expect((await ui.find({ key: 'cal-noplot:D3:choice' }))?.text).toBe('  No labelled decisions: nothing to plot.')
    expect(await ui.find({ type: 'Raster' })).toBe(undefined)
    expect(await ui.find({ type: 'Svg' })).toBe(undefined)
    await ui.press({ key: 'cal:D3' })
    expect(await ui.find({ key: 'cal-threshold:D3:choice' })).toBe(undefined)
    await ui.unmount()
  }
})

test('calibration, partial: a selected point shows its threshold, bins and diagram for the surface', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_PARTIAL })
  await openCalibration($, state)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    expect((await ui.find({ key: 'cal-stats:D1' }))?.text).toBe('   Brier 0.2885 · ask 33% · threshold 0.4 → 0.3')
    expect(await ui.find({ key: 'cal-empty' })).toBe(undefined)
    await ui.press({ key: 'cal:D1' })
    expect((await ui.find({ key: 'cal:D1' }))?.text.startsWith('▾ D1 shadow')).toBe(true)
    expect((await ui.find({ key: 'cal-q:D1:noul' }))?.text).toBe(
      '  noul (destructive_or_irreversible): n 6, labelled 5, unanswered 0, asks 2',
    )
    expect((await ui.find({ key: 'cal-threshold:D1:noul' }))?.text).toBe(
      '  noul: ask at ≥ 0.4, proposed 0.3 (expected cost 1.2 → 0.2)',
    )
    expect((await ui.find({ key: 'cal-bin:D1:noul:9' }))?.text).toBe(
      '  [0.9, 1] 2 decisions · predicted 0.975 · observed 0.5 · hollow',
    )
    expect((await ui.find({ key: 'cal-bin:D1:noul:1' }))?.text).toBe(
      '  [0.1, 0.2) 1 decision · predicted 0.1 · observed 0 · hollow',
    )
    expect(await ui.find({ key: 'cal-bin:D1:noul:0' })).toBe(undefined)
    const raster = await ui.find({ type: 'Raster' })
    const svg = await ui.find({ type: 'Svg' })
    if (surface === 'terminal') {
      expect(svg).toBe(undefined)
      expect(raster?.props.columns).toBe(21)
      expect(raster?.props.rows).toBe(12)
    } else {
      expect(raster).toBe(undefined)
      expect(svg?.props.alt).toBe(
        'D1 noul reliability: predicted against observed for 4 bins with decisions, 4 hollow (fewer than 5 decisions)',
      )
    }
    await ui.press({ key: 'cal:D1' })
    await ui.unmount()
  }
})

test('calibration, one point ready: readiness is ready, short of labels or Brier too high', async ($, on) => {
  const state = host(on, CAL_STATUS, { calibrate: CAL_READY })
  await openCalibration($, state)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D5' }))?.text).toBe('▸ D5 enforce ██████████ 60/50 · ready')
    expect((await ui.find({ key: 'cal-stats:D5' }))?.text).toBe('   Brier 0.0812 · ask 0% · threshold 0.6666')
    expect((await ui.find({ key: 'cal:D7' }))?.text).toBe('▸ D7 unknown ██████████ 55/50 · Brier too high')
    expect((await ui.find({ key: 'cal-stats:D7' }))?.text).toBe('   Brier 0.21 · ask 10% · threshold 0.5')
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    await ui.press({ key: 'cal:D5' })
    expect((await ui.find({ key: 'cal-threshold:D5:score' }))?.text).toBe(
      '  score: doubt_review at ≥ 0.6666, proposed 0.6666 (kept: nothing cheaper)',
    )
    expect((await ui.find({ key: 'cal-bin:D5:score:5' }))?.text.endsWith('· hollow')).toBe(true)
    expect((await ui.find({ key: 'cal-bin:D5:score:7' }))?.text).toBe(
      '  [0.7, 0.8) 14 decisions · predicted 0.74 · observed 0.714',
    )
    expect(await ui.find({ type: 'Button', text: /apply/i })).toBe(undefined)
    await ui.press({ key: 'cal:D5' })
    await ui.unmount()
  }
})

test('the raster and the SVG agree on the bins', async ($, on) => {
  const state = host(on, CAL_STATUS, { calibrate: CAL_READY })
  await openCalibration($, state)
  const terminal = await calibrationPane($, 'terminal')
  await terminal.press({ key: 'cal:D5' })
  const raster = await terminal.find({ type: 'Raster' })
  await terminal.unmount()
  const desktop = await calibrationPane($, 'desktop')
  const svg = await desktop.find({ type: 'Svg' })
  await desktop.unmount()
  expect(raster).not.toBe(undefined)
  expect(svg).not.toBe(undefined)
  const fromRaster = binsOfRaster(raster?.props ?? {})
  const fromSvg = binsOfSvg(String(svg?.props.source))
  const expected: DrawnBin[] = Object.entries(READY_D5_BINS).map(([bin, [count, mean, observed]]) => ({
    bin: Number(bin),
    isHollow: count < 5,
    observedTenths: Math.round(observed * 10),
    isUpperHalf: mean * 10 - Number(bin) >= 0.5,
  }))
  expect(fromRaster).toEqual(expected)
  expect(fromSvg).toEqual(expected)
  expect(fromRaster).toEqual(fromSvg)
  expect(fromSvg.filter(entry => entry.isHollow).map(entry => entry.bin)).toEqual([5])
})

test('calibration refreshes on open and on /anneal calibration refresh, never on a timer', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_EMPTY })
  await openCalibration($, state)
  expect(state.calibrateRuns.length).toBe(1)
  await state.clock.advance(60_000)
  await refreshed($, state)
  await $.tool.call({ tool: 'Bash', command: 'ls' })
  await state.clock.advance(600_000)
  expect(state.calibrateRuns.length).toBe(1)
  state.calibrate.next = CAL_PARTIAL
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe('anneal: calibration refreshed.')
  expect(state.calibrateRuns.length).toBe(2)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D1' }))?.text.includes('5/50')).toBe(true)
    await ui.unmount()
  }
})

test('a failed calibration refresh keeps the last run on screen and says why', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_PARTIAL })
  await openCalibration($, state)
  state.calibrate.next = { exitCode: 70, stdout: '', stderr: 'ERROR the decision log cannot be read\n' }
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe(
    'anneal: calibration refresh failed (ERROR the decision log cannot be read).',
  )
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal-failed' }))?.text).toBe('Last refresh failed: ERROR the decision log cannot be read')
    expect((await ui.find({ key: 'cal:D1' }))?.text.includes('5/50')).toBe(true)
    await ui.unmount()
  }
  expect(state.logged).toEqual([])
})

test('a calibrate that never ran well shows why, on both surfaces', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: { exitCode: 2, stdout: '', stderr: "ERROR unrecognized subcommand 'calibrate'\n" } })
  await openCalibration($, state)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal-note' }))?.text).toBe(
      "anneal calibrate unavailable: ERROR unrecognized subcommand 'calibrate'",
    )
    await ui.unmount()
  }
})

test('on a narrow terminal the calibration is a section of the cockpit pane', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_PARTIAL })
  const answer = await $.command.run({ ...CALIBRATION, presentation: { isFullscreen: false, columns: 80 } })
  expect(answer.text).toBe('anneal: calibration shown in the cockpit pane.')
  expect(state.opened).toEqual(['anneal'])
  await state.clock.settle()
  expect(state.runs[0]?.[4]).toBe('phase,plan,tasks,gates,context,ledger_tail,verdict,clef')
  for (const surface of SURFACES) {
    const ui = await pane($, surface)
    expect(await ui.find({ type: 'Text', text: 'Gates' })).not.toBe(undefined)
    expect(await ui.find({ type: 'Text', text: 'Calibration' })).not.toBe(undefined)
    expect((await ui.find({ key: 'cal-stats:D1' }))?.text).toBe('   Brier 0.2885 · ask 33% · threshold 0.4 → 0.3')
    await ui.unmount()
  }
  // `/anneal` alone then shows the cockpit without the section.
  await $.command.run({ ...ANNEAL, presentation: { isFullscreen: false, columns: 80 } })
  const ui = await pane($, 'terminal')
  expect(await ui.find({ key: 'cal-heading' })).toBe(undefined)
  expect(await ui.find({ type: 'Text', text: 'Gates' })).not.toBe(undefined)
  await ui.unmount()
})

test('/anneal alone keeps the cockpit pane free of the calibration section', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_PARTIAL })
  await $.command.run(ANNEAL)
  await state.clock.settle()
  expect(state.calibrateRuns.length).toBe(0)
  const ui = await pane($, 'terminal')
  expect(await ui.find({ key: 'cal-heading' })).toBe(undefined)
  await ui.unmount()
})

test('where nothing draws, /anneal calibration answers with the calibration as text', async ($, on) => {
  const state = host(on, MID_BUILD, { surfaces: [], calibrate: CAL_PARTIAL })
  const answer = await $.command.run(CALIBRATION)
  expect(state.opened).toEqual([])
  const lines = String(answer.text).split('\n')
  expect(lines[0]).toBe('anneal calibrate:')
  expect(lines[1]).toBe('D1 unknown █░░░░░░░░░ 5/50 · short of labels · Brier 0.2885 · ask 33% · threshold 0.4 → 0.3')
  expect(lines[2]).toBe('  noul: ask at ≥ 0.4, proposed 0.3 (expected cost 1.2 → 0.2)')
  expect(lines.slice(-2)).toEqual([APPLY_NOTE, APPLY_COMMAND])
})

test('/anneal calibration takes only refresh: there is no apply', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_READY })
  const answer = await $.command.run({ ...ANNEAL, args: 'calibration apply' })
  expect(answer.text).toBe('usage: /anneal calibration [refresh]')
  expect(state.calibrateRuns.length).toBe(0)
  expect(state.opened).toEqual([])
})

// ---- Review fixes: launchers, failures, shapes, whole rows ---------------------------------------

/** A promise and the call that settles it, to hold a run until the test lets it answer. */
const gate = (): { readonly promise: Promise<void>; readonly open: () => void } => {
  let open = (): void => undefined
  const promise = new Promise<void>(resolve => {
    open = resolve
  })
  return { promise, open }
}

/**
 * Makes `anneal` the launcher the cockpit tries first, whatever an earlier test left: with
 * `anneal.cmd` unable to start, a run starts only through `anneal`. The calls it made are then
 * forgotten.
 */
const primeLaunchers = async ($: Engine, state: Host): Promise<void> => {
  state.launchers = { 'anneal.cmd': 'ENOENT' }
  await $.command.run(ANNEAL)
  state.launchers = {}
  state.launched.length = 0
  state.runs.length = 0
  state.timeouts.length = 0
}

const MID_BUILD_BAND = 'anneal · build · plan ✓ a1b2c3 · task 3/7 · ctx 58%'

test('a launcher that cannot start passes the run to the next, which is then run first', async ($, on) => {
  const state = host(on, MID_BUILD, { surfaces: [] })
  await primeLaunchers($, state)
  state.launchers = { anneal: 'ENOENT' }
  await refreshed($, state)
  expect(state.launched).toEqual(['anneal', 'anneal.cmd'])
  const ui = await band($, 'terminal')
  expect((await ui.find({ key: 'band' }))?.text).toBe(MID_BUILD_BAND)
  await ui.unmount()
  await refreshed($, state)
  expect(state.launched).toEqual(['anneal', 'anneal.cmd', 'anneal.cmd'])
  expect(state.runs.length).toBe(2)
})

test('a launcher that cannot start (EACCES) passes the text form to the next', async ($, on) => {
  const state = host(on, { exitCode: 0, stdout: 'phase: build\n' }, { surfaces: [] })
  await primeLaunchers($, state)
  state.launchers = { anneal: 'EACCES' }
  expect((await $.command.run(ANNEAL)).text).toBe('phase: build')
  expect(state.launched).toEqual(['anneal', 'anneal.cmd'])
})

test('a run that times out is not retried through the other launcher, and the timeout is the reason', async ($, on) => {
  const state = host(on, MID_BUILD, { surfaces: [] })
  await primeLaunchers($, state)
  state.launchers = { anneal: 'timeout' }
  expect((await $.command.run(ANNEAL)).text).toBe('anneal status unavailable (anneal: $.process.run: timed out after 10000 ms).')
  expect(state.launched).toEqual(['anneal'])
  await refreshed($, state)
  expect(state.launched).toEqual(['anneal', 'anneal'])
  expect(state.runs.length).toBe(0)
  const ui = await pane($, 'terminal')
  expect((await ui.find({ key: 'note' }))?.text).toBe(
    'anneal status unavailable: anneal: $.process.run: timed out after 2000 ms',
  )
  await ui.unmount()
  const shown = await band($, 'terminal')
  expect((await shown.find({ key: 'band' }))?.text).toBe('anneal · status unavailable')
  await shown.unmount()
})

test('no launcher starts: /anneal says anneal status is not built yet, with each start error', async ($, on) => {
  const state = host(on, MID_BUILD, { surfaces: [] })
  await primeLaunchers($, state)
  state.launchers = { anneal: 'ENOENT', 'anneal.cmd': 'EACCES' }
  expect((await $.command.run(ANNEAL)).text).toBe(
    'anneal status is not built yet (anneal: $.process.run: spawn anneal ENOENT; anneal: $.process.run: spawn anneal.cmd EACCES).',
  )
  expect(state.launched).toEqual(['anneal', 'anneal.cmd'])
})

test('an anneal status that runs and fails is unavailable, not unbuilt, in the text form', async ($, on) => {
  host(on, { exitCode: 70, stdout: '', stderr: 'ERROR the ledger cannot be read\n' }, { surfaces: [] })
  expect((await $.command.run(ANNEAL)).text).toBe('anneal status unavailable (ERROR the ledger cannot be read).')
})

test('a clock the host refuses leaves nothing scheduled, so the next ask refreshes', async ($, on) => {
  const refusals = { now: 1 }
  const state = host(on, MID_BUILD, { refuseClock: refusals })
  await turnComplete($)
  await state.clock.settle()
  expect(refusals.now).toBe(0)
  expect(state.runs.length).toBe(0)
  expect(state.logged).toEqual(['anneal cockpit: refresh not scheduled: anneal: $.clock.now: the clock is unavailable'])
  await state.clock.advance(1_000)
  await turnComplete($)
  await state.clock.settle()
  expect(state.runs.length).toBe(1)
  const ui = await band($, 'terminal')
  expect((await ui.find({ key: 'band' }))?.text).toBe(MID_BUILD_BAND)
  await ui.unmount()
})

test('a refresh the host refuses records why as the reading, and the next refresh reads again', async ($, on) => {
  const state = host(on, MID_BUILD)
  state.refuse.panes = 'the pane list is unavailable'
  await refreshed($, state)
  expect(state.runs.length).toBe(0)
  expect(state.logged).toEqual(['anneal cockpit: refresh failed: anneal: $.ui.panes: the pane list is unavailable'])
  const ui = await pane($, 'terminal')
  expect((await ui.find({ key: 'note' }))?.text).toBe(
    'anneal status unavailable: anneal: $.ui.panes: the pane list is unavailable',
  )
  await ui.unmount()
  state.refuse.panes = undefined
  await refreshed($, state)
  expect(state.runs.length).toBe(1)
  const shown = await band($, 'terminal')
  expect((await shown.find({ key: 'band' }))?.text).toBe(MID_BUILD_BAND)
  await shown.unmount()
})

test('a calibration refresh the host refuses records why, and the next refresh runs', async ($, on) => {
  const refusals = { calibration: 1 }
  const state = host(on, MID_BUILD, {
    calibrate: CAL_PARTIAL,
    state: key => {
      if (key !== 'calibration' || refusals.calibration === 0) return undefined
      refusals.calibration -= 1
      return { deny: 'the store is unavailable' }
    },
  })
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe(
    'anneal: calibration refresh failed (anneal: $.state.get: the store is unavailable).',
  )
  expect(state.calibrateRuns.length).toBe(0)
  expect(state.logged).toEqual(['anneal cockpit: calibration refresh failed: anneal: $.state.get: the store is unavailable'])
  const ui = await calibrationPane($, 'terminal')
  expect((await ui.find({ key: 'cal-note' }))?.text).toBe(
    'anneal calibrate unavailable: anneal: $.state.get: the store is unavailable',
  )
  await ui.unmount()
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe('anneal: calibration refreshed.')
  expect(state.calibrateRuns.length).toBe(1)
})

test('a status section whose fields are not the types the cockpit reads is dropped and named', async ($, on) => {
  const state = host(on, {
    exitCode: 0,
    stdout: envelope({
      ...MID_BUILD_DATA,
      phase: { mode: 7 },
      tasks: { done: '3', total: 7 },
      gates: [{ id: 'guard.rm' }],
      context: { percent: '58' },
      ledger_tail: 'oops',
    }),
  })
  await refreshed($, state)
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect((await ui.find({ key: 'band' }))?.text).toBe('anneal · plan ✓ a1b2c3')
    await ui.unmount()
    const opened = await pane($, surface)
    expect((await opened.find({ key: 'status-unreadable' }))?.text).toBe(
      'Unreadable sections, not drawn: phase, tasks, gates, context, ledger_tail',
    )
    expect((await opened.find({ key: 'gates-none' }))?.text).toBe('not read')
    expect((await opened.find({ key: 'tasks' }))?.text).toBe('none')
    expect((await opened.find({ key: 'ledger-none' }))?.text).toBe('No events.')
    expect((await opened.find({ key: 'plan' }))?.text).toBe('docs/plan.md: approved a1b2c3, approved 2026-10-19T10:00:02Z')
    await opened.unmount()
  }
})

test('a well-typed status names no section as unreadable', async ($, on) => {
  const state = host(on, MID_BUILD)
  await refreshed($, state)
  const ui = await pane($, 'terminal')
  expect(await ui.find({ key: 'status-unreadable' })).toBe(undefined)
  await ui.unmount()
})

test('a stored reading that cannot be drawn shows status unreadable, not a blank band', async ($, on) => {
  host(on, MID_BUILD, {
    state: key =>
      key === 'reading' ? { value: { value: { kind: 'read', status: { schema: 1, phase: null, gates: 'x' } }, version: 1 } } : undefined,
  })
  for (const surface of SURFACES) {
    const ui = await band($, surface)
    expect((await ui.find({ key: 'band' }))?.text).toBe('anneal · status unreadable')
    await ui.unmount()
    const opened = await pane($, surface)
    expect((await opened.find({ key: 'note' }))?.text).toBe('anneal status unreadable')
    await opened.unmount()
  }
})

test('a calibration point whose fields are not the types the dashboard reads is dropped and counted', async ($, on) => {
  const state = host(on, MID_BUILD, {
    calibrate: calibrateEnvelope(points({ 'D1:noul': WORKED_D1, 'D2:noul': base => ({ ...base, n: 'none' }) })),
  })
  await openCalibration($, state)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect(await ui.find({ key: 'cal:D2' })).toBe(undefined)
    expect((await ui.find({ key: 'cal-dropped' }))?.text).toBe('1 point not drawn: not the shape this dashboard reads')
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    expect((await ui.find({ key: 'cal:D3' }))?.text).toBe('▸ D3 unknown ░░░░░░░░░░ 0/50 · short of labels')
    await ui.unmount()
  }
})

test('a stored calibration that cannot be drawn says calibrate unreadable, in the pane and as text', async ($, on) => {
  const bad = { kind: 'read', calibration: { points: [{ point: 'D1', verdict: 'stay-shadow', questions: [null] }] } }
  const state = host(on, MID_BUILD, {
    surfaces: [],
    state: key => (key === 'calibration' ? { value: { value: bad, version: 1 } } : undefined),
  })
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal-note' }))?.text).toBe('anneal calibrate unreadable')
    await ui.unmount()
  }
  expect((await $.command.run(CALIBRATION)).text).toBe('anneal calibrate unreadable.')
  expect(state.calibrateRuns.length).toBe(1)
})

test('whole rows: a selected point, after a refresh, after a failed refresh, and a hollow bin', async ($, on) => {
  const state = host(on, CAL_STATUS, { calibrate: CAL_READY })
  await openCalibration($, state)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    await ui.press({ key: 'cal:D1' })
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▾ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    await ui.press({ key: 'cal:D1' })
    await ui.press({ key: 'cal:D5' })
    expect((await ui.find({ key: 'cal-bin:D5:score:5' }))?.text).toBe(
      '  [0.5, 0.6) 3 decisions · predicted 0.55 · observed 0.667 · hollow',
    )
    await ui.press({ key: 'cal:D5' })
    await ui.unmount()
  }
})

test('whole rows: D1 after /anneal calibration refresh, then after a refresh that fails', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_EMPTY })
  await openCalibration($, state)
  state.calibrate.next = CAL_PARTIAL
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe('anneal: calibration refreshed.')
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    await ui.unmount()
  }
  state.calibrate.next = { exitCode: 70, stdout: '', stderr: 'ERROR the decision log cannot be read\n' }
  await $.command.run(CALIBRATION_REFRESH)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    expect((await ui.find({ key: 'cal-stats:D1' }))?.text).toBe('   Brier 0.2885 · ask 33% · threshold 0.4 → 0.3')
    await ui.unmount()
  }
})

test('an ask while anneal calibrate runs is absorbed: one run, and its result is the one stored', async ($, on) => {
  const state = host(on, MID_BUILD, { calibrate: CAL_PARTIAL })
  const held = gate()
  state.hold.calibrate = held.promise
  await openCalibration($, state)
  expect(state.calibrateRuns.length).toBe(1)
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe('anneal: a calibration refresh is already running.')
  expect(state.calibrateRuns.length).toBe(1)
  held.open()
  await state.clock.settle()
  expect(state.calibrateRuns.length).toBe(1)
  for (const surface of SURFACES) {
    const ui = await calibrationPane($, surface)
    expect((await ui.find({ key: 'cal:D1' }))?.text).toBe('▸ D1 shadow █░░░░░░░░░ 5/50 · short of labels')
    expect((await ui.find({ key: 'cal-stats:D1' }))?.text).toBe('   Brier 0.2885 · ask 33% · threshold 0.4 → 0.3')
    await ui.unmount()
  }
  expect((await $.command.run(CALIBRATION_REFRESH)).text).toBe('anneal: calibration refreshed.')
  expect(state.calibrateRuns.length).toBe(2)
})

test('asks while a status refresh runs fold into exactly one more run, whose reading is kept', async ($, on) => {
  const state = host(on, MID_BUILD)
  const held = gate()
  state.hold.status = held.promise
  await refreshed($, state)
  expect(state.runs.length).toBe(1)
  state.answer.next = BLOCKED
  await refreshed($, state)
  await refreshed($, state)
  expect(state.runs.length).toBe(1)
  held.open()
  await state.clock.settle()
  await state.clock.advance(1_000)
  expect(state.runs.length).toBe(2)
  await state.clock.advance(5_000)
  expect(state.runs.length).toBe(2)
  const ui = await band($, 'terminal')
  expect((await ui.find({ key: 'band' }))?.text).toBe(
    'anneal · build · plan ✓ a1b2c3 · task 3/7 · 1 gate open · ctx 58% · guard.rm: rm -rf outside the worktree',
  )
  await ui.unmount()
})
