// The anneal cockpit: the mod half of the plugin (#56).
//
// Hooks enforce, the mod shows and collects. Every gate stays a settings hook in
// `hooks.json` calling the `anneal` CLI; this module holds no logic of its own. It renders what
// `anneal status --json` prints, keeps the last reading in `$.state`, and writes nothing to the
// ledger. anneal works fully without it (`--safe-mode`, `disableAllHooks` or a host without
// mods).
//
// - The band (`AbovePrompt`): one row, `anneal · build · plan ✓ a1b2c3 · task 3/7 · ctx 58%`,
//   in the warning color with the newest open gate's reason while a gate is open. Nothing while
//   there is no ledger.
// - The pane (`/anneal`): gates (each a Button that expands its event), plan, tasks, the verdict
//   for HEAD, Clef's shadow counts and the ledger's tail. It never opens unasked.
// - Refresh: at `session.start`, `turn.complete`, and once a Bash, Edit or Write call has
//   settled; at most once a second, each run of the CLI given 2 s.
// - Where nothing draws (`/anneal` with no surface attached, or a pane no surface places), the
//   command answers with `anneal status`'s text form instead.
// - Calibration (`/anneal calibration`, #63): a pane tab (a section of the cockpit pane on a
//   narrow terminal) with one row per Clef decision point from `anneal calibrate`, and the
//   selected point's reliability diagram, a `Raster` on the terminal and an `Svg` elsewhere. It
//   runs `anneal calibrate` on open and on `/anneal calibration refresh`, never on a timer, and
//   has no apply button: promotion stays `anneal calibrate --apply`, run by a person (#44).

import type { CommandRunInput, CommandRunResult, EngineInterface, Register, RenderElement, RenderInput } from 'claude-code'

import type {
  AnnealCalibration,
  AnnealCalibrationBin,
  AnnealCalibrationPoint,
  AnnealCalibrationQuestion,
  AnnealCalibrationReading,
  AnnealGate,
  AnnealReading,
  AnnealStatus,
} from '../types'

/** The pane's id, which its `ui.render` hook matches as `requestId`. */
export const PANE = 'anneal'

/** How long one refresh's `anneal status --json` may run. */
export const REFRESH_TIMEOUT_MS = 2_000

/** The least time between the starts of two refreshes. */
export const REFRESH_INTERVAL_MS = 1_000

/** How long `/anneal`'s text answer may take: it runs once, when asked. */
const TEXT_TIMEOUT_MS = 10_000

/** How many ledger events a refresh asks for (`--tail`). */
const TAIL = 10

/** The sections the band needs, and the pane's on top while the pane is open. */
const BAND_FIELDS = ['phase', 'plan', 'tasks', 'gates', 'context', 'ledger_tail'] as const
const PANE_FIELDS = [...BAND_FIELDS, 'verdict', 'clef'] as const
/** The band's sections and `clef` (each point's mode), while only the calibration pane is open. */
const CALIBRATION_FIELDS = [...BAND_FIELDS, 'clef'] as const

/** The width a docked pane asks for, and the rows an inline one asks for. */
const DOCK_COLUMNS = 56
const INLINE_ROWS = 18

/** What `/anneal` answers when there is no `anneal status` to run: no launcher starts, or the CLI has no such command. */
export const NOT_BUILT = 'anneal status is not built yet'

/** What `/anneal` answers when `anneal status` started but gave no text: a timeout, cut output or an error. */
export const UNAVAILABLE = 'anneal status unavailable'

/** What the band, the pane and the calibration show for a reading they cannot draw from. */
export const UNREADABLE_BAND = 'anneal · status unreadable'
export const UNREADABLE_PANE = 'anneal status unreadable'
export const UNREADABLE_CALIBRATION = 'anneal calibrate unreadable'

const reading = { plugin: 'anneal', key: 'reading' } as const
const expanded = { plugin: 'anneal', key: 'expanded' } as const

/**
 * The launchers the plugin ships, in the order they are tried: the sh launcher, then the
 * Windows one when the first cannot start.
 */
const launchers = (root: string): readonly string[] => [
  `${root}/libexec/anneal`,
  `${root}/libexec/anneal.cmd`,
]

/**
 * The launcher that started, once one has: from then on it is run first, and the other only
 * when it cannot start, so a run that times out is never retried through the other launcher.
 * `null` until one starts.
 */
let launcherIndex: number | null = null

/** What one run of the CLI came to: its exit and output, or why there is none to read. */
type Ran =
  | { readonly isRun: true; readonly exitCode: number; readonly stdout: string; readonly stderr: string }
  | {
      readonly isRun: false
      readonly reason: string
      /** Whether a launcher started: false when none could (no CLI here), true for a timeout or cut output. */
      readonly hasStarted: boolean
    }

/** The errors of a command that could not start: not there, not executable, not a program. */
const START_FAILURE = /\b(ENOENT|EACCES|ENOEXEC|EPERM|ENOTDIR)\b|\b(cannot|could not|failed to) start\b/i

/**
 * Whether `$.process.run` rejected because the command could not start, rather than because it
 * outlasted its timeout or the host refused it for another reason. The API names no error codes
 * (it "rejects when it cannot start or outlasts `timeoutMs`"), so the spawn error's code or its
 * message decides; anything else counts as started and is not retried.
 */
const isStartFailure = (error: unknown): boolean => {
  const code: unknown = typeof error === 'object' && error !== null ? (error as { code?: unknown }).code : undefined
  if (typeof code === 'string' && START_FAILURE.test(code)) return true

  return START_FAILURE.test(error instanceof Error ? error.message : String(error))
}

/** The text of a rejection, for a reason shown to the person. */
const errorText = (error: unknown): string =>
  error instanceof Error && error.message !== '' ? error.message : String(error)

/**
 * Runs `anneal <args>`: the launcher that started before, else each in order. Only a launcher
 * that cannot start passes the run on to the next; a timeout, or any other rejection, is the
 * run's answer, with its own reason. When none starts the reason is each one's start error.
 */
const runAnneal = async (
  $: EngineInterface,
  args: readonly string[],
  timeoutMs: number,
): Promise<Ran> => {
  const all = launchers($.plugin.root)
  const order = all.map((_, index) => index)
  const tried = launcherIndex === null ? order : [launcherIndex, ...order.filter(index => index !== launcherIndex)]
  const startErrors: string[] = []
  for (const index of tried) {
    const launcher = all[index]
    if (launcher === undefined) continue
    try {
      const ran = await $.process.run([launcher, ...args], { timeoutMs })
      launcherIndex = index
      if (ran.isStdoutTruncated) return { isRun: false, reason: 'its output was cut short', hasStarted: true }

      return { isRun: true, exitCode: ran.exitCode, stdout: ran.stdout, stderr: ran.stderr }
    } catch (error) {
      if (!isStartFailure(error)) {
        launcherIndex = index
        return { isRun: false, reason: errorText(error), hasStarted: true }
      }
      startErrors.push(errorText(error))
    }
  }

  return {
    isRun: false,
    reason: startErrors.length === 0 ? 'no anneal launcher could start' : startErrors.join('; '),
    hasStarted: false,
  }
}

/** The first non-empty line of `text`, trimmed. */
const firstLine = (text: string): string | undefined =>
  text
    .split('\n')
    .find(line => line.trim() !== '')
    ?.trim()

type Fields = Readonly<Record<string, unknown>>

const isObject = (value: unknown): value is Fields =>
  typeof value === 'object' && value !== null && !Array.isArray(value)
const isString = (value: unknown): value is string => typeof value === 'string'
const isNumber = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value)
const isNumberOrNull = (value: unknown): boolean => value === null || isNumber(value)
const isStringOrNull = (value: unknown): boolean => value === null || isString(value)
/** Whether `value` is absent or passes `check`. */
const isOptional = (value: unknown, check: (value: unknown) => boolean): boolean => value === undefined || check(value)
const isListOf = (value: unknown, check: (value: unknown) => boolean): boolean =>
  Array.isArray(value) && value.every(entry => check(entry))

const isGate = (value: unknown): boolean =>
  isObject(value) &&
  isString(value.id) &&
  isString(value.kind) &&
  isString(value.reason) &&
  isNumber(value.event) &&
  isString(value.ts)

const isLedgerEntry = (value: unknown): boolean =>
  isObject(value) && isNumber(value.event) && isString(value.type) && isString(value.summary) && isString(value.ts)

/**
 * Each section the band and pane draw, and the shape of it they read (docs/status.md). A
 * section that does not match is dropped from the reading rather than drawn from.
 */
const STATUS_SECTIONS: readonly (readonly [keyof AnnealStatus, (value: unknown) => boolean])[] = [
  ['phase', value => isObject(value) && isString(value.mode)],
  [
    'plan',
    value =>
      isObject(value) &&
      isString(value.id) &&
      isString(value.status) &&
      isOptional(value.hash, isString) &&
      isOptional(value.approved, approved => isObject(approved) && isString(approved.hash) && isString(approved.ts)),
  ],
  [
    'tasks',
    value =>
      isObject(value) &&
      isNumber(value.done) &&
      isNumber(value.total) &&
      isOptional(value.next, next => isObject(next) && isString(next.id) && isString(next.title)),
  ],
  ['gates', value => isListOf(value, isGate)],
  [
    'verdict',
    value =>
      isObject(value) &&
      isString(value.outcome) &&
      isOptional(value.pr, isNumber) &&
      isOptional(
        value.verdict,
        verdict => isObject(verdict) && isString(verdict.verdict) && isOptional(verdict.fresh, isString),
      ),
  ],
  [
    'clef',
    value =>
      isObject(value) &&
      isListOf(
        value.points,
        point =>
          isObject(point) &&
          isString(point.point) &&
          isString(point.mode) &&
          isNumber(point.answered) &&
          isOptional(point.configured_mode, isString),
      ),
  ],
  ['context', value => isObject(value) && isOptional(value.percent, isNumberOrNull)],
  ['ledger_tail', value => isListOf(value, isLedgerEntry)],
]

/**
 * The sections of a schema 1 status document whose fields have the types the cockpit reads,
 * and the names of those present that do not (dropped, so nothing draws from them).
 */
export const readStatusDocument = (
  data: Fields,
): { readonly status: AnnealStatus; readonly unreadable: readonly string[] } => {
  const kept: Record<string, unknown> = { schema: 1 }
  const unreadable: string[] = []
  for (const [section, check] of STATUS_SECTIONS) {
    const value = data[section]
    if (value === undefined) continue
    if (check(value)) kept[section] = value
    else unreadable.push(section)
  }

  // Every section kept passed its check above, so `kept` has the shape `AnnealStatus` names.
  return { status: kept as AnnealStatus, unreadable }
}

/** The status document inside `anneal status --json`'s envelope, when it is a schema 1 one. */
export const readEnvelope = (
  text: string,
): {
  readonly status: AnnealStatus
  readonly unreadable: readonly string[]
  readonly findings: readonly string[]
} | null => {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return null
  }
  if (!isObject(parsed)) return null
  const { data, findings } = parsed
  if (!isObject(data) || data.schema !== 1) return null
  const keys = Array.isArray(findings)
    ? findings.flatMap(finding => (isObject(finding) && isString(finding.key) ? [finding.key] : []))
    : []

  return { ...readStatusDocument(data), findings: keys }
}

/** One refresh's reading of the CLI; `store` decides whether a failure replaces anything. */
const readStatus = async ($: EngineInterface, fields: readonly string[]): Promise<AnnealReading> => {
  const args = ['status', '--json', '--fields', fields.join(','), '--tail', String(TAIL)]
  const ran = await runAnneal($, args, REFRESH_TIMEOUT_MS)
  if (!ran.isRun) return { kind: 'unavailable', reason: ran.reason }
  const envelope = readEnvelope(ran.stdout)
  if (ran.exitCode === 0 && envelope !== null) {
    return envelope.unreadable.length === 0
      ? { kind: 'read', status: envelope.status }
      : { kind: 'read', status: envelope.status, unreadable: envelope.unreadable }
  }
  if (ran.exitCode === 2 && envelope?.findings.includes('status/no-ledger') === true) {
    return { kind: 'absent' }
  }
  const said = firstLine(ran.stderr) ?? firstLine(ran.stdout)

  return { kind: 'unavailable', reason: said ?? `exit ${ran.exitCode}` }
}

/** Stores a reading; a failure keeps the last good one, and an unchanged one writes nothing. */
const store = async ($: EngineInterface, next: AnnealReading): Promise<void> => {
  const held = await $.state.get(reading)
  const current = held.value ?? { kind: 'unread' }
  if (next.kind === 'unavailable' && current.kind === 'read') return
  if (JSON.stringify(current) === JSON.stringify(next)) return
  await $.state.set(reading, next)
}

/** Expands a gate row, or folds it when it is the one expanded; read when pressed, not drawn. */
const toggle = async ($: EngineInterface, event: number): Promise<void> => {
  const held = await $.state.get(expanded)
  await $.state.set(expanded, held.value === event ? null : event, { ifVersion: held.version })
}

/** The refresh's pacing, the module's own: lost on a reload, as only the reading must not be. */
let lastStartedAt = Number.NEGATIVE_INFINITY
let isScheduled = false
let isRunning = false
let isAskedAgain = false

/**
 * One refresh: the sections the band needs, the pane's too while the pane is open. It rejects
 * when the host refuses a call it needs; its caller records that as the reading's reason.
 */
const refreshNow = async ($: EngineInterface): Promise<void> => {
  isScheduled = false
  if (isRunning) {
    isAskedAgain = true
    return
  }
  isRunning = true
  try {
    try {
      lastStartedAt = await $.clock.now()
      const panes = new Set((await $.ui.panes()).map(pane => pane.id))
      const fields = panes.has(PANE) ? PANE_FIELDS : panes.has(CALIBRATION_PANE) ? CALIBRATION_FIELDS : BAND_FIELDS
      await store($, await readStatus($, fields))
    } finally {
      isRunning = false
    }
  } finally {
    // An ask made while this one ran is answered whether or not this one failed.
    if (isAskedAgain) {
      isAskedAgain = false
      await refresh($)
    }
  }
}

/**
 * Records why a refresh failed: the host refused a call it needs. The reason goes to the debug
 * log and becomes the reading's (a good reading stays, as `store` keeps it); the transcript is
 * left alone, and the next refresh tries again.
 */
const refreshFailed = async ($: EngineInterface, error: unknown): Promise<void> => {
  const reason = errorText(error)
  $.ui.log(`anneal cockpit: refresh failed: ${reason}`, { to: 'debug' })
  try {
    await store($, { kind: 'unavailable', reason })
  } catch (stored) {
    $.ui.log(`anneal cockpit: the refresh's failure was not recorded: ${errorText(stored)}`, { to: 'debug' })
  }
}

/**
 * Asks for a refresh: now when the last one started a second or more ago, else one second after
 * it. Asks made while one waits fold into it. It never rejects: a clock the host refuses leaves
 * nothing scheduled, so the next ask tries again.
 */
export const refresh = async ($: EngineInterface): Promise<void> => {
  if (isScheduled) return
  isScheduled = true
  let isTimerSet = false
  try {
    const now = await $.clock.now()
    const wait = Math.max(0, lastStartedAt + REFRESH_INTERVAL_MS - now)
    $.clock.after(wait, () => {
      refreshNow($).catch(error => refreshFailed($, error))
    })
    isTimerSet = true
  } catch (error) {
    $.ui.log(`anneal cockpit: refresh not scheduled: ${errorText(error)}`, { to: 'debug' })
  } finally {
    if (!isTimerSet) isScheduled = false
  }
}

/** `text` cut to `columns` characters, an ellipsis at the end when it was cut. */
const fit = (text: string, columns: number): string => {
  const chars = Array.from(text)
  if (chars.length <= columns) return text
  if (columns <= 1) return chars.slice(0, Math.max(0, columns)).join('')

  return `${chars.slice(0, columns - 1).join('')}…`
}

/** The short form of a `sha256:` hash: its first six hex digits. */
const shortHash = (hash: string | undefined): string | undefined =>
  hash === undefined ? undefined : hash.replace(/^sha256:/, '').slice(0, 6)

/**
 * The gates still open: the blocks recorded after the newest event of the ledger's tail that is
 * not itself a block. Status lists the gates that recently blocked (docs/status.md); a block
 * nothing has followed but other blocks is the one the session is still stuck behind. Without
 * the tail (a section that could not be read) no gate is called open.
 */
export const openGates = (status: AnnealStatus): readonly AnnealGate[] => {
  const tail = status.ledger_tail
  const gates = status.gates ?? []
  if (tail === undefined) return []
  const progressed = tail
    .filter(entry => entry.type !== 'gate_blocked' && entry.type !== 'stop_blocked')
    .reduce((newest, entry) => Math.max(newest, entry.event), 0)

  return gates.filter(gate => gate.event > progressed)
}

/** The band's one row for a status document, and whether a gate holds it. */
export const bandLine = (status: AnnealStatus): { readonly text: string; readonly isBlocked: boolean } => {
  const parts = ['anneal']
  if (status.phase !== undefined) parts.push(status.phase.mode)
  if (status.plan !== undefined) {
    const hash = shortHash(status.plan.approved?.hash ?? status.plan.hash)
    if (status.plan.status === 'approved') parts.push(hash === undefined ? 'plan ✓' : `plan ✓ ${hash}`)
    else if (status.plan.status === 'stale') parts.push('plan stale')
    else parts.push('plan unapproved')
  }
  if (status.tasks !== undefined) parts.push(`task ${status.tasks.done}/${status.tasks.total}`)
  const open = openGates(status)
  if (open.length > 0) parts.push(`${open.length} gate${open.length === 1 ? '' : 's'} open`)
  const percent = status.context?.percent
  if (typeof percent === 'number') parts.push(`ctx ${Math.round(percent)}%`)
  const newest = open[open.length - 1]
  if (newest !== undefined) parts.push(`${newest.id}: ${newest.reason}`)

  return { text: parts.join(' · '), isBlocked: newest !== undefined }
}

/**
 * The text `/anneal` answers where nothing draws: `anneal status`'s text form (it prints one
 * with a ledger or without), else one line saying why there is none.
 */
export const statusText = async ($: EngineInterface): Promise<string> => {
  const ran = await runAnneal($, ['status'], TEXT_TIMEOUT_MS)
  if (!ran.isRun) return `${ran.hasStarted ? UNAVAILABLE : NOT_BUILT} (${ran.reason}).`
  const text = ran.stdout.trim()
  if ((ran.exitCode === 0 || ran.exitCode === 2) && text !== '') return text
  const said = firstLine(ran.stderr) ?? `exit ${ran.exitCode}`
  // A CLI built before `status` existed rejects it as a usage error; anything else is a failure.
  const isMissing = ran.exitCode === 2 && /unrecognized subcommand/i.test(said)

  return `${isMissing ? NOT_BUILT : UNAVAILABLE} (${said}).`
}

// ---- Calibration (#63) --------------------------------------------------------------------------

/** The calibration pane's id: a second pane, which the surface shows as a tab beside `anneal`. */
export const CALIBRATION_PANE = 'anneal-calibration'

/** How long one `anneal calibrate` may run: it reads the whole decision log, once, when asked. */
export const CALIBRATION_TIMEOUT_MS = 10_000

/**
 * Below this many terminal columns `/anneal calibration` draws a section of the cockpit pane
 * instead of a tab of its own.
 */
export const NARROW_COLUMNS = 100

/** The labelled decisions a question needs before it may be promoted (docs/calibrate.md). */
export const LABELS_FOR_PROMOTION = 50

/** A bin with fewer decisions than this is drawn hollow. */
export const HOLLOW_BELOW = 5

/** What `/anneal calibration` answers to arguments it does not take. */
export const CALIBRATION_USAGE = 'usage: /anneal calibration [refresh]'

/** The cells of the labelled-decisions bar. */
const BAR_CELLS = 10

const calibration = { plugin: 'anneal', key: 'calibration' } as const
const selectedPoint = { plugin: 'anneal', key: 'selected_point' } as const
const calibrationSection = { plugin: 'anneal', key: 'calibration_section' } as const

const isCalibrationBin = (value: unknown): boolean =>
  isObject(value) &&
  isNumber(value.lower) &&
  isNumber(value.upper) &&
  isNumber(value.count) &&
  isNumberOrNull(value.mean_probability) &&
  isNumberOrNull(value.observed_frequency)

const isCalibrationThreshold = (value: unknown): boolean =>
  isObject(value) &&
  isStringOrNull(value.band) &&
  isNumberOrNull(value.current) &&
  isNumberOrNull(value.proposed) &&
  isOptional(value.current_expected_cost, isNumberOrNull) &&
  isOptional(value.proposed_expected_cost, isNumberOrNull) &&
  isOptional(value.no_proposal, isStringOrNull)

const isCalibrationQuestion = (value: unknown): boolean =>
  isObject(value) &&
  isString(value.question) &&
  isString(value.subject) &&
  isNumber(value.n) &&
  isNumber(value.labelled) &&
  isOptional(value.labelled_via_mod, isNumber) &&
  isNumber(value.unanswered) &&
  isNumber(value.asks) &&
  isNumberOrNull(value.brier) &&
  isNumberOrNull(value.ask_rate) &&
  isListOf(value.bins, isCalibrationBin) &&
  isCalibrationThreshold(value.threshold) &&
  isListOf(value.reasons, isString)

/** Whether a point has the fields the rows and diagrams read, each of its questions too. */
const isCalibrationPoint = (value: unknown): value is AnnealCalibrationPoint =>
  isObject(value) &&
  isString(value.point) &&
  isOptional(value.verdict, isString) &&
  isListOf(value.questions, isCalibrationQuestion)

/**
 * The points and proposals path of an `anneal calibrate` envelope (docs/calibrate.md, Output),
 * or `null` when the text is not one. A point whose fields, or any of whose questions' fields,
 * do not have the types the dashboard reads is dropped and counted in `dropped`.
 */
export const readCalibration = (text: string): AnnealCalibration | null => {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return null
  }
  if (!isObject(parsed) || parsed.command !== 'calibrate') return null
  const data = parsed.data
  if (!isObject(data) || !Array.isArray(data.points)) return null
  const all: readonly unknown[] = data.points
  const points = all.filter(isCalibrationPoint)
  const dropped = all.length - points.length

  return {
    points,
    ...(isString(data.proposals) ? { proposals: data.proposals } : {}),
    ...(dropped > 0 ? { dropped } : {}),
  }
}

/** One run of `anneal calibrate`, read. */
const runCalibrate = async ($: EngineInterface): Promise<AnnealCalibrationReading> => {
  const ran = await runAnneal($, ['calibrate'], CALIBRATION_TIMEOUT_MS)
  if (!ran.isRun) return { kind: 'unavailable', reason: ran.reason }
  const read = readCalibration(ran.stdout)
  if (ran.exitCode === 0 && read !== null) return { kind: 'read', calibration: read }

  return { kind: 'unavailable', reason: firstLine(ran.stderr) ?? firstLine(ran.stdout) ?? `exit ${ran.exitCode}` }
}

/** Whether a run of `anneal calibrate` is under way; an ask meanwhile starts none. */
let isCalibrating = false

/** What is stored of `anneal calibrate`, or `unread`. */
const heldCalibration = async ($: EngineInterface): Promise<AnnealCalibrationReading> =>
  (await $.state.get(calibration)).value ?? { kind: 'unread' }

/**
 * A failed calibration refresh, as stored: the last good run with why this one failed, else
 * `unavailable` with the reason.
 */
const calibrationFailure = (current: AnnealCalibrationReading, reason: string): AnnealCalibrationReading =>
  current.kind === 'read'
    ? { kind: 'read', calibration: current.calibration, failed: reason }
    : { kind: 'unavailable', reason }

/**
 * Records a calibration refresh that failed outside a run (the host refused a call): the reason
 * goes to the debug log and, where the store can be read, to the reading. Resolves to that
 * reading.
 */
const calibrationFailed = async ($: EngineInterface, error: unknown): Promise<AnnealCalibrationReading> => {
  const reason = errorText(error)
  $.ui.log(`anneal cockpit: calibration refresh failed: ${reason}`, { to: 'debug' })
  try {
    const current = await heldCalibration($)
    const kept = calibrationFailure(current, reason)
    if (JSON.stringify(kept) !== JSON.stringify(current)) await $.state.set(calibration, kept)
    return kept
  } catch (stored) {
    $.ui.log(`anneal cockpit: the calibration failure was not recorded: ${errorText(stored)}`, { to: 'debug' })
    return { kind: 'unavailable', reason }
  }
}

/**
 * Runs `anneal calibrate` and stores what it printed: a failure after a good run keeps that run
 * and says why, and an unchanged result writes nothing. Resolves to what is stored; a run under
 * way absorbs the ask and resolves to what is stored now. It never rejects: a host that refuses
 * a call is recorded as the reason.
 */
export const refreshCalibration = async ($: EngineInterface): Promise<AnnealCalibrationReading> => {
  if (isCalibrating) return heldCalibration($).catch(error => calibrationFailed($, error))
  isCalibrating = true
  try {
    const current = await heldCalibration($)
    const next = await runCalibrate($)
    const kept = next.kind === 'unavailable' ? calibrationFailure(current, next.reason) : next
    if (JSON.stringify(kept) !== JSON.stringify(current)) await $.state.set(calibration, kept)

    return kept
  } catch (error) {
    return calibrationFailed($, error)
  } finally {
    isCalibrating = false
  }
}

/** Shows a point's diagram, or folds it when it is the one shown; read when pressed. */
const selectPoint = async ($: EngineInterface, point: string): Promise<void> => {
  const held = await $.state.get(selectedPoint)
  await $.state.set(selectedPoint, held.value === point ? null : point, { ifVersion: held.version })
}

/** The question a point's row reports: its first with bands (the one that acts), else its first. */
export const primaryQuestion = (point: AnnealCalibrationPoint): AnnealCalibrationQuestion | undefined =>
  point.questions.find(question => question.threshold.band !== null) ?? point.questions[0]

/** How close a point is to promotion, as `anneal calibrate`'s verdict and reasons say. */
export type Readiness = 'ready' | 'short of labels' | 'Brier too high' | 'ask rate too high' | 'stay shadow'

/**
 * A point's readiness. It is `ready` only when calibrate calls it `eligible-to-enforce`;
 * otherwise the first reason of the questions that count (those with bands and decisions, else
 * the row's question), in the order labels, Brier, ask rate. Nothing is recomputed here.
 */
export const readiness = (point: AnnealCalibrationPoint): Readiness => {
  if (point.verdict === 'eligible-to-enforce') return 'ready'
  const counted = point.questions.filter(question => question.threshold.band !== null && question.n > 0)
  const primary = primaryQuestion(point)
  const asked = counted.length > 0 ? counted : primary === undefined ? [] : [primary]
  const reasons = new Set(asked.flatMap(question => question.reasons))
  if (reasons.has('too_few_labelled')) return 'short of labels'
  if (reasons.has('brier_above_bound')) return 'Brier too high'
  if (reasons.has('ask_rate_at_or_above_bound')) return 'ask rate too high'

  return 'stay shadow'
}

/** A number cut to `digits` decimals with trailing zeros dropped, or `–` for none. */
const decimal = (value: number | null | undefined, digits: number): string =>
  typeof value === 'number' && Number.isFinite(value) ? String(Number(value.toFixed(digits))) : '–'

/** A rate as a whole percentage, or `–` for none. */
const percent = (value: number | null | undefined): string =>
  typeof value === 'number' && Number.isFinite(value) ? `${Math.round(value * 100)}%` : '–'

/** `labelled` out of the 50 promotion needs, as a bar of ten cells. */
export const labelBar = (labelled: number): string => {
  const filled = Math.min(BAR_CELLS, Math.floor((Math.max(0, labelled) * BAR_CELLS) / LABELS_FOR_PROMOTION))

  return `${'█'.repeat(filled)}${'░'.repeat(BAR_CELLS - filled)}`
}

/** The mode the status document gives a point (`clef.points[].mode`), or `unknown`. */
export const modeOf = (status: AnnealReading, point: string): string =>
  (status.kind === 'read' ? status.status.clef?.points.find(entry => entry.point === point)?.mode : undefined) ??
  'unknown'

/** A question's threshold now and as proposed, short: `threshold 0.4 → 0.3`, or `threshold 0.4`. */
const shortThreshold = (question: AnnealCalibrationQuestion): string => {
  const { current, proposed } = question.threshold
  if (current === null) return 'no threshold'
  if (proposed === null || proposed === current) return `threshold ${decimal(current, 4)}`

  return `threshold ${decimal(current, 4)} → ${decimal(proposed, 4)}`
}

/** A question's threshold now and as proposed, in full, with the expected costs. */
export const thresholdText = (question: AnnealCalibrationQuestion): string => {
  const threshold = question.threshold
  if (threshold.current === null) {
    return `${question.question}: no threshold (${threshold.no_proposal ?? 'no bands'})`
  }
  const now = `${question.question}: ${threshold.band ?? 'act'} at ≥ ${decimal(threshold.current, 4)}`
  if (threshold.proposed === null) return `${now}, no proposal (${threshold.no_proposal ?? 'none'})`
  if (threshold.proposed === threshold.current) {
    return `${now}, proposed ${decimal(threshold.proposed, 4)} (kept: nothing cheaper)`
  }

  return `${now}, proposed ${decimal(threshold.proposed, 4)} (expected cost ${decimal(
    threshold.current_expected_cost,
    4,
  )} → ${decimal(threshold.proposed_expected_cost, 4)})`
}

/**
 * A point's row, in two lines so it fits a docked pane: the mode, labelled decisions toward 50
 * and readiness; then the Brier score, ask rate and threshold now and as proposed.
 */
export const pointLines = (
  point: AnnealCalibrationPoint,
  mode: string,
): { readonly head: string; readonly stats: string } => {
  const question = primaryQuestion(point)
  if (question === undefined) return { head: `${point.point} ${mode}`, stats: 'no questions' }
  const labelled = question.labelled

  return {
    head: `${point.point} ${mode} ${labelBar(labelled)} ${labelled}/${LABELS_FOR_PROMOTION} · ${readiness(point)}`,
    stats: [`Brier ${decimal(question.brier, 4)}`, `ask ${percent(question.ask_rate)}`, shortThreshold(question)].join(
      ' · ',
    ),
  }
}

/** A point's row on one line, as the text form prints it. */
export const pointRow = (point: AnnealCalibrationPoint, mode: string): string => {
  const lines = pointLines(point, mode)

  return `${lines.head} · ${lines.stats}`
}

/** One bin as a reliability diagram draws it: where it sits, how many decisions, filled or not. */
export type PlotPoint = {
  /** Its index among the bins, 0 for `[0.0, 0.1)`. */
  readonly bin: number
  readonly count: number
  /** The mean predicted probability. */
  readonly predicted: number
  /** The fraction of positives. */
  readonly observed: number
  /** Fewer than five decisions. */
  readonly isHollow: boolean
}

const unit = (value: number): number => Math.min(1, Math.max(0, value))

/** The bins a diagram draws: every one with decisions, in bin order. */
export const plotPoints = (bins: readonly AnnealCalibrationBin[]): readonly PlotPoint[] =>
  bins.flatMap((bin, index) =>
    bin.count > 0 && typeof bin.mean_probability === 'number' && typeof bin.observed_frequency === 'number'
      ? [
          {
            bin: index,
            count: bin.count,
            predicted: unit(bin.mean_probability),
            observed: unit(bin.observed_frequency),
            isHollow: bin.count < HOLLOW_BELOW,
          },
        ]
      : [],
  )

/** The raster's plot rows: observed 1.0 at the top down to 0.0, in tenths; the axis is below. */
export const PLOT_ROWS = 11
/** The glyphs of a filled and a hollow bin. */
export const FILLED_GLYPH = '●'
export const HOLLOW_GLYPH = '○'
const DIAGONAL_GLYPH = 0x00b7
const AXIS_V = 0x2502
const AXIS_H = 0x2500
const AXIS_CORNER = 0x2514
const SPACE = 0x20
/** The terminal's default color, and the diagram's two. */
const DEFAULT_COLOR = 0x01000000
const AXIS_COLOR = 0x808080
const POINT_COLOR = 0x3b82f6

/** The raster row an observed frequency lands on. */
export const rasterRow = (observed: number): number => PLOT_ROWS - 1 - Math.round(unit(observed) * (PLOT_ROWS - 1))

/**
 * The raster column a bin lands on: column 0 is the axis, and bin k owns columns 2k+1 and 2k+2,
 * the second when its mean predicted probability is in the upper half of the bin.
 */
export const rasterColumn = (point: PlotPoint, binCount: number): number =>
  1 + 2 * point.bin + (point.predicted * binCount - point.bin >= 0.5 ? 1 : 0)

const BASE64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** Standard padded base64 of `bytes`. */
const base64 = (bytes: readonly number[]): string => {
  const out: string[] = []
  for (let index = 0; index < bytes.length; index += 3) {
    const a = bytes[index] ?? 0
    const b = bytes[index + 1] ?? 0
    const c = bytes[index + 2] ?? 0
    const word = (a << 16) | (b << 8) | c
    out.push(BASE64.charAt((word >> 18) & 63), BASE64.charAt((word >> 12) & 63))
    out.push(index + 1 < bytes.length ? BASE64.charAt((word >> 6) & 63) : '=')
    out.push(index + 2 < bytes.length ? BASE64.charAt(word & 63) : '=')
  }

  return out.join('')
}

/**
 * A reliability diagram as a `Raster`'s grid: observed up the left axis, predicted along the
 * bottom, the dotted diagonal where they agree, and each bin with decisions as `●`, or `○` when
 * it holds fewer than five.
 */
export const rasterOf = (
  bins: readonly AnnealCalibrationBin[],
): { readonly columns: number; readonly rows: number; readonly cells: string } => {
  const binCount = Math.max(1, bins.length)
  const width = 2 * binCount
  const columns = width + 1
  const rows = PLOT_ROWS + 1
  const glyphs: number[] = Array.from({ length: columns * rows }, () => SPACE)
  const colors: number[] = Array.from({ length: columns * rows }, () => DEFAULT_COLOR)
  const put = (column: number, row: number, glyph: number, color: number): void => {
    if (column < 0 || column >= columns || row < 0 || row >= rows) return
    glyphs[row * columns + column] = glyph
    colors[row * columns + column] = color
  }
  for (let row = 0; row < PLOT_ROWS; row += 1) put(0, row, AXIS_V, AXIS_COLOR)
  put(0, PLOT_ROWS, AXIS_CORNER, AXIS_COLOR)
  for (let column = 1; column < columns; column += 1) put(column, PLOT_ROWS, AXIS_H, AXIS_COLOR)
  for (let column = 0; column < width; column += 1) {
    put(1 + column, rasterRow((column + 0.5) / width), DIAGONAL_GLYPH, AXIS_COLOR)
  }
  for (const point of plotPoints(bins)) {
    const glyph = (point.isHollow ? HOLLOW_GLYPH : FILLED_GLYPH).codePointAt(0) ?? SPACE
    put(rasterColumn(point, binCount), rasterRow(point.observed), glyph, POINT_COLOR)
  }
  const bytes: number[] = []
  const word = (value: number): void => {
    bytes.push(value & 0xff, (value >>> 8) & 0xff, (value >>> 16) & 0xff, (value >>> 24) & 0xff)
  }
  for (let index = 0; index < glyphs.length; index += 1) {
    word(glyphs[index] ?? SPACE)
    word(colors[index] ?? DEFAULT_COLOR)
    word(DEFAULT_COLOR)
  }

  return { columns, rows, cells: base64(bytes) }
}

/** The SVG diagram's size and its plot square's place in it, in CSS pixels. */
export const SVG_WIDTH = 250
export const SVG_HEIGHT = 250
export const SVG_PLOT = 200
export const SVG_LEFT = 36
export const SVG_TOP = 10

const px = (value: number): string => String(Math.round(value * 100) / 100)

/** The SVG x of a predicted probability, and the y of an observed frequency. */
export const svgX = (predicted: number): number => SVG_LEFT + unit(predicted) * SVG_PLOT
export const svgY = (observed: number): number => SVG_TOP + (1 - unit(observed)) * SVG_PLOT

/**
 * The same reliability diagram as an SVG document: each bin a circle (`class="bin filled"` or
 * `"bin hollow"`, `data-bin` its index, `data-count` its decisions) labelled with its count.
 */
export const svgOf = (bins: readonly AnnealCalibrationBin[]): string => {
  const right = SVG_LEFT + SVG_PLOT
  const bottom = SVG_TOP + SVG_PLOT
  const middle = SVG_TOP + SVG_PLOT / 2
  const axis = '#8a8a8a'
  const color = '#3b82f6'
  const parts = [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${SVG_WIDTH}" height="${SVG_HEIGHT}" viewBox="0 0 ${SVG_WIDTH} ${SVG_HEIGHT}" font-family="sans-serif" font-size="10">`,
    `<rect x="${SVG_LEFT}" y="${SVG_TOP}" width="${SVG_PLOT}" height="${SVG_PLOT}" fill="none" stroke="${axis}"/>`,
    `<line x1="${SVG_LEFT}" y1="${bottom}" x2="${right}" y2="${SVG_TOP}" stroke="${axis}" stroke-dasharray="3 3"/>`,
    `<text x="${SVG_LEFT}" y="${bottom + 14}" text-anchor="middle" fill="${axis}">0</text>`,
    `<text x="${right}" y="${bottom + 14}" text-anchor="middle" fill="${axis}">1</text>`,
    `<text x="${SVG_LEFT + SVG_PLOT / 2}" y="${bottom + 28}" text-anchor="middle" fill="${axis}">predicted</text>`,
    `<text x="${SVG_LEFT - 6}" y="${bottom}" text-anchor="end" fill="${axis}">0</text>`,
    `<text x="${SVG_LEFT - 6}" y="${SVG_TOP + 8}" text-anchor="end" fill="${axis}">1</text>`,
    `<text x="12" y="${middle}" text-anchor="middle" fill="${axis}" transform="rotate(-90 12 ${middle})">observed</text>`,
  ]
  for (const point of plotPoints(bins)) {
    const x = svgX(point.predicted)
    const y = svgY(point.observed)
    parts.push(
      `<circle class="bin ${point.isHollow ? 'hollow' : 'filled'}" data-bin="${point.bin}" data-count="${point.count}" cx="${px(x)}" cy="${px(y)}" r="5" fill="${point.isHollow ? 'none' : color}" stroke="${color}" stroke-width="1.5"/>`,
      `<text x="${px(x + 8)}" y="${px(y + 3)}" fill="${axis}">${point.count}</text>`,
    )
  }
  parts.push('</svg>')

  return parts.join('')
}

/** What a diagram says, for a reader that cannot see it. */
const diagramAlt = (point: string, question: string, bins: readonly AnnealCalibrationBin[]): string => {
  const drawn = plotPoints(bins)
  const hollow = drawn.filter(entry => entry.isHollow).length

  return `${point} ${question} reliability: predicted against observed for ${drawn.length} bin${
    drawn.length === 1 ? '' : 's'
  } with decisions, ${hollow} hollow (fewer than ${HOLLOW_BELOW} decisions)`
}

/** One bin's line under its diagram. */
const binText = (bin: AnnealCalibrationBin, isLast: boolean): string =>
  `  [${decimal(bin.lower, 2)}, ${decimal(bin.upper, 2)}${isLast ? ']' : ')'} ${bin.count} decision${
    bin.count === 1 ? '' : 's'
  } · predicted ${decimal(bin.mean_probability, 3)} · observed ${decimal(bin.observed_frequency, 3)}${
    bin.count < HOLLOW_BELOW ? ' · hollow' : ''
  }`

/** The note under the rows: proposals only, and how a person promotes (#44). No button does it. */
export const APPLY_NOTE = 'Proposals only, nothing applied. A person promotes with'
export const APPLY_COMMAND = '  anneal calibrate --apply --thresholds <file>'

/**
 * `/anneal calibration`'s text where nothing draws: a line per point and per question's
 * threshold, else one line saying why there is none.
 */
export const calibrationText = (held: AnnealCalibrationReading, status: AnnealReading): string => {
  if (held.kind === 'unread') return 'anneal calibrate has not run.'
  if (held.kind === 'unavailable') return `anneal calibrate unavailable (${held.reason}).`
  const lines = ['anneal calibrate:']
  for (const point of held.calibration.points) {
    lines.push(pointRow(point, modeOf(status, point.point)))
    for (const question of point.questions) lines.push(`  ${thresholdText(question)}`)
  }
  if (held.failed !== undefined) lines.push(`Last refresh failed: ${held.failed}; the lines above are the run before it.`)
  lines.push(APPLY_NOTE, APPLY_COMMAND)

  return lines.join('\n')
}

/** A `ui.render` ask for a pane. */
type PaneRender = Extract<RenderInput, { component: 'Pane' }>

/** The calibration dashboard's tree, for its own pane or a section of the cockpit pane. */
const calibrationTree = async ($: EngineInterface, e: PaneRender, columns: number): Promise<RenderElement> => {
  const { Box, Button, Text } = $.ui.resolve(e)
  const { value: held = { kind: 'unread' } } = await $.state.get(calibration)
  const { value: selected = null } = await $.state.get(selectedPoint)
  const { value: status = { kind: 'unread' } } = await $.state.get(reading)
  const row = (key: string, text: string, dim = false) => (
    <Box key={key}>
      <Text dimColor={dim} wrap="truncate-end">
        {fit(text, columns)}
      </Text>
    </Box>
  )
  const heading = (
    <Box key="cal-heading">
      <Text weight="bold" wrap="truncate-end">
        {fit('Calibration', columns)}
      </Text>
    </Box>
  )
  // The diagram's element is picked by the surface: only the terminal has `Raster`, and every
  // other surface has `Svg` instead.
  const diagram = (key: string, point: string, question: AnnealCalibrationQuestion) => {
    if (e.surface === 'terminal') {
      const { Raster } = $.ui.resolve(e)
      const raster = rasterOf(question.bins)
      return <Raster key={key} columns={raster.columns} rows={raster.rows} cells={raster.cells} />
    }
    const { Svg } = $.ui.resolve(e)

    return (
      <Box key={key}>
        <Svg
          source={svgOf(question.bins)}
          alt={diagramAlt(point, question.question, question.bins)}
          width={SVG_WIDTH}
          height={SVG_HEIGHT}
        />
      </Box>
    )
  }

  if (held.kind === 'unread') {
    return (
      <Box flexDirection="column">
        {heading}
        {row('cal-note', 'Reading anneal calibrate…', true)}
      </Box>
    )
  }
  if (held.kind === 'unavailable') {
    return (
      <Box flexDirection="column">
        {heading}
        {row('cal-note', `anneal calibrate unavailable: ${held.reason}`, true)}
      </Box>
    )
  }
  const points = held.calibration.points
  const isEmpty = points.every(point => point.questions.every(question => question.n === 0))

  return (
    <Box flexDirection="column">
      {heading}
      {held.failed !== undefined && row('cal-failed', `Last refresh failed: ${held.failed}`, true)}
      {isEmpty && row('cal-empty', 'No decisions logged yet: every point is short of labels.', true)}
      {held.calibration.dropped !== undefined &&
        row(
          'cal-dropped',
          `${held.calibration.dropped} point${held.calibration.dropped === 1 ? '' : 's'} not drawn: not the shape this dashboard reads`,
          true,
        )}
      {points.map(point => {
        const lines = pointLines(point, modeOf(status, point.point))
        return (
        <Box key={`cal-box:${point.point}`} flexDirection="column">
          <Button key={`cal:${point.point}`} plain onPress={() => selectPoint($, point.point)}>
            {fit(`${selected === point.point ? '▾' : '▸'} ${lines.head}`, columns)}
          </Button>
          {row(`cal-stats:${point.point}`, `   ${lines.stats}`, true)}
          {selected === point.point &&
            point.questions.map(question => {
              const id = `${point.point}:${question.question}`
              const isPlotted = plotPoints(question.bins).length > 0
              return (
                <Box key={`cal-question:${id}`} flexDirection="column">
                  {row(
                    `cal-q:${id}`,
                    `  ${question.question} (${question.subject}): n ${question.n}, labelled ${question.labelled}, unanswered ${question.unanswered}, asks ${question.asks}`,
                  )}
                  {row(`cal-threshold:${id}`, `  ${thresholdText(question)}`)}
                  {isPlotted
                    ? diagram(`cal-diagram:${id}`, point.point, question)
                    : row(`cal-noplot:${id}`, '  No labelled decisions: nothing to plot.', true)}
                  {question.bins.map((bin, index) =>
                    bin.count > 0
                      ? row(`cal-bin:${id}:${index}`, binText(bin, index === question.bins.length - 1), true)
                      : null,
                  )}
                </Box>
              )
            })}
        </Box>
        )
      })}
      {row('cal-apply', APPLY_NOTE, true)}
      {row('cal-apply-command', APPLY_COMMAND, true)}
      {held.calibration.proposals !== undefined &&
        row('cal-proposals', `proposals: ${held.calibration.proposals}`, true)}
    </Box>
  )
}

/** The calibration as a section of the cockpit pane, or one line saying it cannot be drawn. */
const calibrationSectionTree = async ($: EngineInterface, e: PaneRender, columns: number): Promise<RenderElement> => {
  try {
    return await calibrationTree($, e, columns)
  } catch (error) {
    $.ui.log(`anneal cockpit: calibration not drawn: ${errorText(error)}`, { to: 'debug' })
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box key="cal-note">
        <Text dimColor wrap="truncate-end">
          {fit(UNREADABLE_CALIBRATION, columns)}
        </Text>
      </Box>
    )
  }
}

/** `calibrationText`, or one line saying the stored run cannot be read. */
const calibrationTextOrNote = (held: AnnealCalibrationReading, status: AnnealReading): string => {
  try {
    return calibrationText(held, status)
  } catch {
    return `${UNREADABLE_CALIBRATION}.`
  }
}

/** `/anneal calibration [refresh]`. */
const calibrationCommand = async (
  $: EngineInterface,
  e: CommandRunInput,
  rest: readonly string[],
): Promise<CommandRunResult> => {
  const isRefresh = rest.length === 1 && rest[0] === 'refresh'
  if (rest.length > 0 && !isRefresh) return { text: CALIBRATION_USAGE }
  const surfaces = await $.session.surfaces()
  const statusNow = async (): Promise<AnnealReading> => (await $.state.get(reading)).value ?? { kind: 'unread' }
  if (isRefresh) {
    const held = await refreshCalibration($)
    if (surfaces.length === 0) return { text: calibrationTextOrNote(held, await statusNow()) }
    if (held.kind === 'read' && held.failed === undefined) return { text: 'anneal: calibration refreshed.' }
    const reason = held.kind === 'read' ? held.failed : held.kind === 'unavailable' ? held.reason : 'no run finished'

    return { text: `anneal: calibration refresh failed (${reason ?? 'unknown'}).` }
  }
  if (surfaces.length > 0) {
    const isNarrow = e.presentation.columns < NARROW_COLUMNS
    const shown = await $.state.get(calibrationSection)
    if (shown.value !== isNarrow) await $.state.set(calibrationSection, isNarrow)
    const id = isNarrow ? PANE : CALIBRATION_PANE
    const size = e.presentation.isFullscreen ? { columns: DOCK_COLUMNS } : { rows: INLINE_ROWS }
    const opened = await $.ui.open({ id, title: isNarrow ? 'anneal' : 'calibration', ...size })
    if (opened.isPlaced) {
      await refresh($)
      void refreshCalibration($).catch(error => calibrationFailed($, error))
      return { text: isNarrow ? 'anneal: calibration shown in the cockpit pane.' : 'anneal: calibration pane opened.' }
    }
    await $.ui.close({ id })
  }

  return { text: calibrationTextOrNote(await refreshCalibration($), await statusNow()) }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'anneal',
      description:
        "Open anneal's cockpit pane (gates, plan, tasks, verdict, Clef, ledger); `calibration` for Clef's calibration",
      argumentHint: '[calibration [refresh]]',
    })
    await refresh($)

    return next(e)
  })

  on('turn.complete', async ($, e, next) => {
    const result = await next(e)
    await refresh($)

    return result
  })

  // Observes, never rewrites or refuses: the call runs as it would without the cockpit, and a
  // failure here hands back what the call settled to (`next(e)` replays it, nothing runs twice).
  on('tool.call', { tool: ['Bash', 'Edit', 'Write'] }, async ($, e, next) => {
    const result = await next(e)
    await refresh($)

    return result
  }).catch(($, e, next) => next(e))

  on('command.run', { command: 'anneal' }, async ($, e) => {
    const words = e.args.trim().split(/\s+/).filter(word => word !== '')
    if (words[0] === 'calibration') return calibrationCommand($, e, words.slice(1))
    // `/anneal` alone is the cockpit without the calibration section a narrow terminal added.
    const section = await $.state.get(calibrationSection)
    if (section.value === true) await $.state.set(calibrationSection, false)
    const surfaces = await $.session.surfaces()
    if (surfaces.length > 0) {
      const size = e.presentation.isFullscreen ? { columns: DOCK_COLUMNS } : { rows: INLINE_ROWS }
      const opened = await $.ui.open({ id: PANE, title: 'anneal', ...size })
      if (opened.isPlaced) {
        await refresh($)
        return { text: 'anneal: cockpit pane opened.' }
      }
      await $.ui.close({ id: PANE })
    }

    return { text: await statusText($) }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.props.hasSurvey) return next(e)
    const { value = { kind: 'unread' } } = await $.state.get(reading)
    if (value.kind === 'unread' || value.kind === 'absent') return next(e)
    const { Box, Text } = $.ui.resolve(e)
    const columns = e.props.bodyColumns
    if (value.kind === 'unavailable') {
      return (
        <Box key="band">
          <Text dimColor wrap="truncate-end">
            {fit('anneal · status unavailable', columns)}
          </Text>
        </Box>
      )
    }
    // A reading this module did not check (one kept from before a reload) may not have the shape
    // `bandLine` reads: the band then says so rather than drawing nothing.
    let line: { readonly text: string; readonly isBlocked: boolean }
    try {
      line = bandLine(value.status)
    } catch (error) {
      $.ui.log(`anneal cockpit: band not drawn: ${errorText(error)}`, { to: 'debug' })
      line = { text: UNREADABLE_BAND, isBlocked: false }
    }

    return (
      <Box key="band">
        <Text color={line.isBlocked ? 'warning' : undefined} dimColor={!line.isBlocked} wrap="truncate-end">
          {fit(line.text, columns)}
        </Text>
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Button, Text } = $.ui.resolve(e)
    const columns = e.props.bodyColumns
    const { value = { kind: 'unread' } } = await $.state.get(reading)
    const { value: open = null } = await $.state.get(expanded)
    const { value: isSection = false } = await $.state.get(calibrationSection)
    const section = isSection ? await calibrationSectionTree($, e, columns) : null
    const row = (key: string, text: string, dim = false) => (
      <Box key={key}>
        <Text dimColor={dim} wrap="truncate-end">
          {fit(text, columns)}
        </Text>
      </Box>
    )
    const heading = (key: string, text: string) => (
      <Box key={key}>
        <Text weight="bold" wrap="truncate-end">
          {fit(text, columns)}
        </Text>
      </Box>
    )

    if (value.kind === 'unread') {
      return (
        <Box flexDirection="column">
          {row('note', 'Reading anneal status…', true)}
          {section}
        </Box>
      )
    }
    if (value.kind === 'absent') {
      return (
        <Box flexDirection="column">
          {row('note', 'No ledger here yet: nothing has been recorded.', true)}
          {section}
        </Box>
      )
    }
    if (value.kind === 'unavailable') {
      return (
        <Box flexDirection="column">
          {row('note', `anneal status unavailable: ${value.reason}`, true)}
          {section}
        </Box>
      )
    }

    try {
      return statusTree(value)
    } catch (error) {
      $.ui.log(`anneal cockpit: pane not drawn: ${errorText(error)}`, { to: 'debug' })
      return (
        <Box flexDirection="column">
          {row('note', UNREADABLE_PANE, true)}
          {section}
        </Box>
      )
    }

    function statusTree(value: Extract<AnnealReading, { kind: 'read' }>): RenderElement {
      const status = value.status
      const stillOpen = new Set(openGates(status).map(gate => gate.event))
      const gates = status.gates ?? []
      const plan = status.plan
      const tasks = status.tasks
      const verdict = status.verdict
      const clef = status.clef
      const tail = status.ledger_tail ?? []

      return (
        <Box flexDirection="column">
          {value.unreadable !== undefined &&
            row('status-unreadable', `Unreadable sections, not drawn: ${value.unreadable.join(', ')}`, true)}
          {heading('h-gates', 'Gates')}
          {status.gates === undefined && row('gates-none', 'not read', true)}
          {status.gates !== undefined && gates.length === 0 && row('gates-none', 'No gate has blocked.', true)}
          {gates
            .slice()
            .reverse()
            .map(gate => (
              <Box key={`gate-box:${gate.event}`} flexDirection="column">
                <Button
                  key={`gate:${gate.event}`}
                  plain
                  onPress={() => toggle($, gate.event)}
                >
                  {fit(`${open === gate.event ? '▾' : '▸'} #${gate.event} ${gate.id}`, Math.max(1, columns - 8))}
                  <Text dimColor>{stillOpen.has(gate.event) ? ' open' : ' passed'}</Text>
                </Button>
                {open === gate.event && row(`gate-kind:${gate.event}`, `  ${gate.kind} at ${gate.ts}`, true)}
                {open === gate.event && row(`gate-reason:${gate.event}`, `  ${gate.reason}`)}
              </Box>
            ))}

          {heading('h-plan', 'Plan')}
          {plan === undefined
            ? row('plan', 'none (not in build mode)', true)
            : row(
                'plan',
                `${plan.id}: ${plan.status}${plan.hash === undefined ? '' : ` ${shortHash(plan.hash)}`}${
                  plan.approved === undefined ? '' : `, approved ${plan.approved.ts}`
                }`,
              )}

          {heading('h-tasks', 'Tasks')}
          {tasks === undefined
            ? row('tasks', 'none', true)
            : row(
                'tasks',
                `${tasks.done}/${tasks.total} done${tasks.next === undefined ? '' : `, next ${tasks.next.id} ${tasks.next.title}`}`,
              )}

          {heading('h-verdict', 'Verdict for HEAD')}
          {verdict === undefined
            ? row('verdict', 'none', true)
            : row(
                'verdict',
                `${verdict.outcome}${verdict.pr === undefined ? '' : ` (PR #${verdict.pr})`}${
                  verdict.verdict === undefined ? '' : `: ${verdict.verdict.verdict}, fresh ${verdict.verdict.fresh ?? 'unknown'}`
                }`,
              )}

          {heading('h-clef', 'Clef')}
          {clef === undefined
            ? row('clef', 'none', true)
            : clef.points.map(point =>
                row(
                  `clef:${point.point}`,
                  `${point.point} ${point.mode}, ${point.answered} answered${
                    point.configured_mode === undefined || point.configured_mode === point.mode
                      ? ''
                      : ` (configured ${point.configured_mode})`
                  }`,
                ),
              )}

          {heading('h-ledger', 'Ledger')}
          {tail.length === 0 && row('ledger-none', 'No events.', true)}
          {tail.map(entry => row(`ledger:${entry.event}`, `#${entry.event} ${entry.summary} (${entry.ts})`, true))}

          {section}
        </Box>
      )
    }
  })

  on('ui.render', { component: 'Pane', requestId: CALIBRATION_PANE }, async ($, e) => {
    try {
      return await calibrationTree($, e, e.props.bodyColumns)
    } catch (error) {
      $.ui.log(`anneal cockpit: calibration not drawn: ${errorText(error)}`, { to: 'debug' })
      const { Box, Text } = $.ui.resolve(e)
      return (
        <Box flexDirection="column">
          <Box key="cal-note">
            <Text dimColor wrap="truncate-end">
              {fit(UNREADABLE_CALIBRATION, e.props.bodyColumns)}
            </Text>
          </Box>
        </Box>
      )
    }
  })
}
