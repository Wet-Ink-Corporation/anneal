// The anneal mod's type contract (`plugin.json` names it as `types`).
//
// The mod holds no state of its own: every value it shows comes from `anneal status --json` or
// `anneal calibrate`, and every action it takes is an `anneal` subcommand. What it keeps in
// `$.state` is the last reading of each, which gate row the pane has expanded, which decision
// point's diagram is shown and where the calibration is drawn, so a drawing
// redraws without asking the CLI again and survives a hot reload. Nothing here is a source of
// truth; the ledger is.
//
// The section types follow docs/status.md (`schema: 1`). Fields are only ever added within a
// schema, so each type is open: a reader ignores what it does not know.

/** `phase`: the worktree's anneal mode and its playbook. */
export type AnnealPhase = {
  readonly mode: string
  readonly playbook?: { readonly name: string; readonly requires: readonly string[] }
  readonly event?: number
  readonly plan?: string
  readonly tree?: string
  readonly clean?: boolean
}

/** `plan`: build mode's plan and whether its current bytes are the approved ones. */
export type AnnealPlan = {
  readonly id: string
  readonly status: 'approved' | 'stale' | 'none' | (string & {})
  readonly hash?: string
  readonly approved?: { readonly hash: string; readonly event: number; readonly ts: string }
}

/** `tasks`: build mode's plan's tasks done and in all, and the first not done. */
export type AnnealTasks = {
  readonly done: number
  readonly total: number
  readonly next?: { readonly id: string; readonly title: string }
}

/** One entry of `gates`: a `gate_blocked` or `stop_blocked` event. */
export type AnnealGate = {
  readonly id: string
  readonly kind: string
  readonly reason: string
  readonly event: number
  readonly ts: string
}

/** `verdict`: the review verdict for `HEAD`. */
export type AnnealVerdict = {
  readonly pr?: number
  readonly head?: string
  readonly base?: string
  readonly outcome: string
  readonly valid: boolean
  readonly verdict?: {
    readonly event: number
    readonly verdict: string
    readonly source?: string
    readonly sha?: string
    readonly fresh?: string
  }
  readonly stale?: number
}

/** One Clef decision point's answered decisions and mode. */
export type AnnealClefPoint = {
  readonly point: string
  readonly answered: number
  readonly configured_mode?: string
  readonly mode: string
}

/** `clef`: the Clef decision points D1 to D7. */
export type AnnealClef = {
  readonly configured: boolean
  readonly points: readonly AnnealClefPoint[]
}

/** `context`: the newest session's context usage. */
export type AnnealContext = {
  readonly used_tokens: number | null
  readonly window?: number
  readonly window_source?: string
  readonly percent: number | null
  readonly level: string | null
}

/** One entry of `ledger_tail`. */
export type AnnealLedgerEntry = {
  readonly event: number
  readonly type: string
  readonly summary: string
  readonly actor?: string
  readonly ts: string
}

/** One reading of `anneal status --json`'s `data` (#58). Every section is optional. */
export type AnnealStatus = {
  /** The status document's schema version; this mod reads version 1. */
  readonly schema: 1
  readonly phase?: AnnealPhase
  readonly plan?: AnnealPlan
  readonly tasks?: AnnealTasks
  readonly gates?: readonly AnnealGate[]
  readonly verdict?: AnnealVerdict
  readonly clef?: AnnealClef
  readonly context?: AnnealContext
  readonly ledger_tail?: readonly AnnealLedgerEntry[]
}

/**
 * What the cockpit last learned from the CLI:
 *
 * - `unread`: no refresh has finished yet;
 * - `absent`: `anneal status` exited 2 with its `status/no-ledger` finding (no ledger here);
 * - `read`: the last good status document, without any section whose fields did not have the
 *   types the cockpit reads (`unreadable` names those);
 * - `unavailable`: the CLI could not be run or printed no status document, and there was no
 *   good reading to keep (a failure after a good reading keeps that reading).
 */
export type AnnealReading =
  | { readonly kind: 'unread' }
  | { readonly kind: 'absent' }
  | {
      readonly kind: 'read'
      readonly status: AnnealStatus
      /** The sections the CLI printed whose fields did not have the types the cockpit reads; dropped. */
      readonly unreadable?: readonly string[]
    }
  | { readonly kind: 'unavailable'; readonly reason: string }

/**
 * One reliability bin of `anneal calibrate` (docs/calibrate.md, Output): `[lower, upper)`, the
 * last closed at 1.0. An empty bin has `null` for its mean and frequency.
 */
export type AnnealCalibrationBin = {
  readonly lower: number
  readonly upper: number
  readonly count: number
  readonly mean_probability: number | null
  readonly observed_frequency: number | null
}

/** A question's lowest band: the threshold now, and as proposed. */
export type AnnealCalibrationThreshold = {
  readonly band: string | null
  readonly pointer: string | null
  readonly current: number | null
  readonly ceiling: number | null
  readonly false_accept: 'below_threshold' | 'at_or_above_threshold' | (string & {})
  readonly current_expected_cost: number | null
  readonly proposed: number | null
  readonly proposed_expected_cost: number | null
  readonly no_proposal: 'no_bands' | 'no_labels' | (string & {}) | null
}

/** One question of one decision point. */
export type AnnealCalibrationQuestion = {
  readonly question: 'noul' | 'choice' | 'score' | (string & {})
  readonly subject: string
  readonly n: number
  readonly labelled: number
  /** Of `labelled`, the decisions whose last label came `via: mod` (the mod's buttons). */
  readonly labelled_via_mod?: number
  readonly unanswered: number
  readonly unusable_labels: number
  readonly brier: number | null
  readonly bins: readonly AnnealCalibrationBin[]
  readonly asks: number
  readonly ask_rate: number | null
  readonly threshold: AnnealCalibrationThreshold
  readonly verdict: 'stay-shadow' | 'eligible-to-enforce' | (string & {})
  readonly reasons: readonly ('too_few_labelled' | 'brier_above_bound' | 'ask_rate_at_or_above_bound' | (string & {}))[]
}

/** One decision point (D1 to D7) of `anneal calibrate`. */
export type AnnealCalibrationPoint = {
  readonly point: string
  readonly verdict: 'stay-shadow' | 'eligible-to-enforce' | (string & {})
  readonly questions: readonly AnnealCalibrationQuestion[]
}

/** What the cockpit keeps of `anneal calibrate`'s `data`: the points and where the proposal is. */
export type AnnealCalibration = {
  readonly points: readonly AnnealCalibrationPoint[]
  /** The proposals file the run wrote (`<anneal home>/decisions/proposals.json`). */
  readonly proposals?: string
  /** How many points the run printed whose fields did not have the types the dashboard reads; dropped. */
  readonly dropped?: number
}

/**
 * What the cockpit last learned from `anneal calibrate`, which runs only when asked
 * (`/anneal calibration`, `/anneal calibration refresh`), never on a timer:
 *
 * - `unread`: no run has finished yet;
 * - `read`: the last good run; `failed` says why a later run printed none;
 * - `unavailable`: the CLI could not be run or printed no envelope, and there was no good run.
 */
export type AnnealCalibrationReading =
  | { readonly kind: 'unread' }
  | { readonly kind: 'read'; readonly calibration: AnnealCalibration; readonly failed?: string }
  | { readonly kind: 'unavailable'; readonly reason: string }

declare module 'claude-code' {
  interface PluginState {
    anneal: {
      /** What the cockpit last learned from `anneal status --json`. */
      reading: AnnealReading
      /** The ledger position of the gate row the pane shows expanded, or `null` for none. */
      expanded: number | null
      /** What the cockpit last learned from `anneal calibrate`. */
      calibration: AnnealCalibrationReading
      /** The decision point whose reliability diagram is shown, or `null` for none. */
      selected_point: string | null
      /** True when the calibration is drawn as a section of the cockpit pane (narrow terminals). */
      calibration_section: boolean
    }
  }
}
