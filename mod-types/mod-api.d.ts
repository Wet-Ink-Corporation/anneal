// A minimal, hand-written declaration of the Claude Code mods API, in this repository's own
// words, covering only what plugin/hooks/cockpit.tsx and plugin/tests/cockpit.test.ts use.
//
// It mirrors the documented API (https://code.claude.com/docs/en/plugins/mods/reference) as of
// Claude Code 2.1.303. It is not Claude Code's own declaration file, which is not under this
// repository's licence and is not shipped in the `@anthropic-ai/claude-code` npm package
// (2.1.303 ships only `sdk-tools.d.ts`). The real engine checks the mod in CI: the job in
// docs/mods.md runs `claude plugin validate --strict plugin` and `claude plugin test plugin`
// on the pinned Claude Code, so a shape this file gets wrong fails there. Extend it only as the
// mod starts using more of the API; where it is looser than the engine (a tool's input, an
// element's props), the engine's own check is the one that counts.

/** What JSX compiles against in a hooks module: `h(tag, props, ...children)`. */
declare const h: (
  tag: (props: never) => import('claude-code').RenderElement,
  props: object | null,
  ...children: readonly import('claude-code').RenderChild[]
) => import('claude-code').RenderElement

/** The fragment tag `<>...</>` compiles to. */
declare const Fragment: (props: { children?: import('claude-code').RenderChild }) => import('claude-code').RenderElement

declare namespace JSX {
  type Element = import('claude-code').RenderElement
  interface ElementChildrenAttribute {
    children: {}
  }
  interface IntrinsicElements {}
}

declare module 'claude-code' {
  /** Session state values a mod declares in its contract (plugin.json `types`). */
  export interface PluginState {}

  // ---- Drawing -------------------------------------------------------------------------------

  /** A drawn element, as plain data the surface validates and paints. */
  export type RenderElement = {
    readonly type: string
    readonly props: Readonly<Record<string, unknown>>
    readonly children?: readonly unknown[]
  }

  /** What may stand as an element's child: an element, text, nothing, or a list of them. */
  export type RenderChild =
    | RenderElement
    | string
    | number
    | boolean
    | null
    | undefined
    | readonly RenderChild[]

  /** A surface's element, called by JSX with its props and children. */
  export type ElementConstructor<P> = (props: P & { children?: RenderChild }) => RenderElement

  /** A color: one of the theme's named keys (`warning`, `error`, ...) or a color string. */
  export type Color = 'text' | 'subtle' | 'success' | 'error' | 'warning' | (string & {})

  /** The few `Box` props the cockpit sets. */
  export type BoxProps = {
    /** Its address in a drawing; a test finds the box by it. */
    key?: string
    flexDirection?: 'row' | 'column'
  }

  /** The few `Text` props the cockpit sets. */
  export type TextProps = {
    color?: Color
    dimColor?: boolean
    weight?: 'regular' | 'medium' | 'semibold' | 'bold'
    wrap?: 'wrap' | 'truncate-end'
  }

  /** What a press hands a Button's `onPress`: at least the surface it came from. */
  export type PressArgument = { readonly surface: string }

  /** The few `Button` props the cockpit sets: one press target, its children drawn as its label. */
  export type ButtonProps = {
    key?: string
    label?: string
    plain?: true
    onPress: (e: PressArgument) => void
  }

  /** The elements every surface the cockpit draws on has. */
  export type SharedElements = {
    Box: ElementConstructor<BoxProps>
    Text: ElementConstructor<TextProps>
    Button: ElementConstructor<ButtonProps>
  }

  /**
   * `Raster`'s props: the terminal's fixed grid of cells. `cells` is standard padded base64 of
   * `columns * rows` little-endian u32 triplets `[codePoint, foreground, background]`, row-major;
   * a color is `0x00RRGGBB`, or `0x01000000` for the terminal's default.
   */
  export type RasterProps = {
    /** Its address in the drawing, unique among the drawing's Rasters. */
    key: string
    /** 1 to 512. */
    columns: number
    /** 1 to 256. */
    rows: number
    cells: string
  }

  /** `Svg`'s props: the remote surfaces' vector leaf, drawn isolated (as an image). */
  export type SvgProps = {
    /** The document, `<svg ...>...</svg>`, at most 131072 characters. */
    source: string
    /** What the drawing says, for a reader that cannot see it. */
    alt: string
    /** CSS pixels. */
    width?: number
    height?: number
  }

  /** The surfaces a render hook can be asked to draw on. */
  export type RenderSurface = 'terminal' | 'desktop' | 'vscode' | 'mobile'

  /**
   * Each surface's element table, as far as the cockpit uses it: only the terminal has `Raster`,
   * and every other surface has `Svg` instead.
   */
  export type Elements = {
    terminal: SharedElements & { Raster: ElementConstructor<RasterProps> }
    desktop: SharedElements & { Svg: ElementConstructor<SvgProps> }
    vscode: SharedElements & { Svg: ElementConstructor<SvgProps> }
    mobile: SharedElements & { Svg: ElementConstructor<SvgProps> }
  }

  /** Where a site's window sits over a tree taller than it. */
  export type SiteScroll = { readonly offset: number; readonly bodyRows: number }

  /** Which transcript is on screen beside a site: the main one, or an agent's. */
  export type SiteView = { readonly agentId?: string }

  /** The band above the prompt. */
  export type AbovePromptProps = {
    /** True while a survey holds the band; a hook leaves it alone then. */
    readonly hasSurvey: boolean
    readonly isWorking: boolean
    readonly maxRows: number
    /** The cells the band's tree is laid out in. */
    readonly bodyColumns: number
    readonly scroll: SiteScroll
    readonly view: SiteView
  }

  /** A pane a plugin opened with `$.ui.open`. */
  export type PaneProps = {
    readonly title: string
    readonly isFocused: boolean
    /** The cells across the pane's body. */
    readonly bodyColumns: number
    /** `dock` beside the transcript, or `inline` above the prompt. */
    readonly placement: 'dock' | 'inline'
    readonly scroll: SiteScroll
    readonly view: SiteView
  }

  /** The props of each component the cockpit draws. */
  export type RenderPropsOf = {
    AbovePrompt: AbovePromptProps
    Pane: PaneProps
  }

  /** The size a surface draws into, and whether it docks a pane. */
  export type RenderViewport = {
    readonly columns: number
    readonly rows: number
    readonly isFullscreen?: boolean
  }

  /**
   * One `ui.render` ask: which component, on which surface, with which props. A union over both
   * the component and the surface, so testing `e.surface` narrows what `$.ui.resolve(e)` hands
   * back.
   */
  export type RenderInput = {
    [C in keyof RenderPropsOf]: {
      [S in RenderSurface]: {
        readonly surface: S
        readonly component: C
        readonly requestId: string
        readonly viewport?: RenderViewport
        readonly props: RenderPropsOf[C]
      }
    }[RenderSurface]
  }[keyof RenderPropsOf]

  // ---- Panes, state, clock, processes, commands ------------------------------------------------

  /** `$.ui.open`'s argument. */
  export type PaneOpenArgs = {
    id: string
    title?: string
    /** Body rows wanted while seated inline above the prompt. */
    rows?: number
    /** Body columns wanted while docked beside a fullscreen transcript. */
    columns?: number
  }

  /** Whether an opened pane is drawn now, and why not when it is not. */
  export type UiOpenResult = { isPlaced: true } | { isPlaced: false; reason: string }

  /** One of this plugin's open panes. */
  export type UiPane = {
    id: string
    title: string
    isShown: boolean
    isFocused: boolean
    isPlaced: boolean
  }

  /** A `$.state` value's address: the plugin and the key its contract declares. */
  export type StateRef<P extends keyof PluginState, K extends keyof PluginState[P]> = {
    readonly plugin: P
    readonly key: K
  }

  /** A `$.state` read: the value (absent before the first write) and its version. */
  export type StateRead<T> = { value?: T; version: number }

  /** A pending `$.clock.after` timer. */
  export type Timer = { cancel: () => void }

  /** Options of `$.process.run`. */
  export type ProcessRunInit = {
    cwd?: string
    env?: Record<string, string>
    stdin?: string
    /** Milliseconds before the child is killed and the call rejects; 30 s by default. */
    timeoutMs?: number
  }

  /** How a child run by `$.process.run` ended, each stream cut at 4 MiB. */
  export type ProcessRunResult = {
    exitCode: number
    stdout: string
    stderr: string
    isStdoutTruncated: boolean
    isStderrTruncated: boolean
  }

  /** A slash command a mod serves, as `$.command.register` takes it. */
  export type CommandSpec = {
    name: string
    description: string
    argumentHint?: string
    immediate?: true
  }

  /** `command.run`'s input, as the engine stamps it. */
  export type CommandRunInput = {
    command: string
    args: string
    origin: { kind: 'composer' } | { kind: 'plugin'; name: string }
    /** Whether the answer shows fullscreen (where a pane docks) and how wide the terminal is. */
    presentation: { isFullscreen: boolean; columns: number }
  }

  /** What a `command.run` hook answers: the line shown as the command's output. */
  export type CommandRunResult = {
    text?: string
    context?: readonly string[]
    exitCode?: number
  }

  /** The `$` every hook receives: only the namespaces the cockpit calls. */
  export interface EngineInterface {
    plugin: { readonly name: string; readonly root: string }
    process: {
      /** Runs a host command by argv; rejects when it cannot start or outlasts `timeoutMs`. */
      run: (argv: readonly string[], init?: ProcessRunInit) => Promise<ProcessRunResult>
    }
    command: {
      register: (command: CommandSpec) => Promise<{ command: string }>
      run: (input: { command: string; args?: string }) => Promise<CommandRunResult>
    }
    session: {
      /** Every surface the session draws on; empty where nothing draws (`-p`, no client). */
      surfaces: () => Promise<readonly RenderSurface[]>
    }
    ui: {
      /** The element table of the surface a `ui.render` ask draws on. */
      resolve: <E extends RenderInput>(e: E) => Elements[E['surface']]
      open: (pane: PaneOpenArgs) => Promise<UiOpenResult>
      close: (pane: { id: string }) => Promise<void>
      panes: () => Promise<readonly UiPane[]>
      log: (text: string, options?: { to?: 'transcript' | 'debug' }) => void
    }
    clock: {
      /** Milliseconds since the epoch. */
      now: () => Promise<number>
      /** Calls `fn` once after `ms`; a timer outlives the hook that set it. */
      after: (ms: number, fn: () => void) => Timer
    }
    state: {
      /** Reads a value; read while drawing, it redraws the reader when the value changes. */
      get: <P extends keyof PluginState, K extends keyof PluginState[P]>(
        ref: StateRef<P, K>,
      ) => Promise<StateRead<PluginState[P][K]>>
      /** Writes one of this plugin's own values; refused while a render hook draws. */
      set: <P extends keyof PluginState, K extends keyof PluginState[P]>(
        ref: StateRef<P, K>,
        value: PluginState[P][K],
        options?: { ifVersion?: number },
      ) => Promise<{ isSet: boolean; version: number }>
    }
  }

  // ---- Events --------------------------------------------------------------------------------

  /** A refusal or a value, as a hook on a `$` call answers it. */
  export type ValueOrDeny<V> = { value: V; deny?: undefined } | { deny: string; value?: undefined }

  /** A tool call: the tool's name and its arguments (looser here than in the engine). */
  export type ToolCallInput = { tool: string; tool_use_id?: string; [argument: string]: unknown }

  /** A tool call's outcome: its result, or a refusal. */
  export type ToolCallResult = { result: unknown; text?: string; isError?: true } | { deny: string }

  /** `turn.complete`'s input (the fields the cockpit's test passes). */
  export type TurnCompleteInput = {
    answer: string
    durationMs: number
    isAborted: boolean
    turnId: string
    reason: 'answer' | 'aborted' | 'error'
  }

  /** `session.start`'s input. */
  export type SessionStartInput = {
    cwd: string
    surface: RenderSurface | null
    isInteractive: boolean
  }

  /** Each event the cockpit or its test hooks: its input, then its result. */
  export type HookedEvents = {
    'session.start': [SessionStartInput, { cwd: string }]
    'turn.complete': [TurnCompleteInput, { text: string }]
    'tool.call': [ToolCallInput, ToolCallResult]
    'command.run': [CommandRunInput, CommandRunResult]
    'ui.render': [RenderInput, RenderElement]
    'process.run': [{ argv: readonly string[]; init?: ProcessRunInit }, ValueOrDeny<ProcessRunResult>]
    'session.surfaces': [Readonly<Record<string, never>>, ValueOrDeny<readonly RenderSurface[]>]
    'ui.open': [PaneOpenArgs, ValueOrDeny<UiOpenResult>]
    'ui.close': [{ id: string }, ValueOrDeny<void>]
    'ui.panes': [Readonly<Record<string, never>>, ValueOrDeny<readonly UiPane[]>]
    'ui.log': [{ text: string; to?: 'transcript' | 'debug' }, ValueOrDeny<void>]
    'command.register': [CommandSpec, ValueOrDeny<{ command: string }>]
    /** Only a test hooks these, to stand for a host that refuses them. */
    'state.get': [{ plugin: string; key: string }, ValueOrDeny<StateRead<unknown>>]
    'clock.now': [Readonly<Record<string, never>>, ValueOrDeny<number>]
    'clock.after': [{ ms: number }, ValueOrDeny<void>]
  }

  export type Next<In, Out> = (e: In) => Promise<Out>

  /** The input a matcher keeps: a `component` or `tool` it names narrows the input to it. */
  export type Narrowed<I, M> = I extends unknown
    ? M extends { component: infer C }
      ? I extends { component: C }
        ? I
        : never
      : I
    : never

  export type Hook<N extends keyof HookedEvents, In = HookedEvents[N][0]> = (
    $: EngineInterface,
    e: In,
    next: Next<HookedEvents[N][0], HookedEvents[N][1]>,
  ) => Promise<HookedEvents[N][1]>

  /** A matcher on an event's fields: a value is `===`, a list is any of its values. */
  export type Matcher<I> = {
    readonly [K in keyof I]?: I[K] | readonly I[K][]
  }

  /** What `on` returns: `.catch` sets the hook's error handler. */
  export type Registration<N extends keyof HookedEvents> = {
    catch: (handler: Hook<N>) => void
  }

  /** Adds a hook, optionally behind a matcher on the event's fields. */
  export type On = {
    <N extends keyof HookedEvents>(event: N, hook: Hook<N>): Registration<N>
    <N extends keyof HookedEvents, const M extends Matcher<HookedEvents[N][0]>>(
      event: N,
      matcher: M,
      hook: Hook<N, Narrowed<HookedEvents[N][0], M>>,
    ): Registration<N>
  }

  /** The hooks module's one export. */
  export type Register = (on: On, options: Readonly<Record<string, unknown>>) => void
}

declare module 'claude-code/testing' {
  import type {
    CommandRunInput,
    CommandRunResult,
    On,
    RenderElement,
    RenderPropsOf,
    RenderSurface,
    RenderViewport,
    SessionStartInput,
    ToolCallInput,
    ToolCallResult,
    TurnCompleteInput,
  } from 'claude-code'

  /** One element of a mounted drawing, read as plain data. */
  export type FoundElement = {
    type: string
    key: string | undefined
    props: Record<string, unknown>
    /** What it shows: its text children in document order. */
    text: string
  }

  /** What `find` matches an element on: its tag, its key, the text it shows. */
  export type ElementQuery = { type?: string; key?: string; text?: string | RegExp }

  /** A component instance the test had a surface draw through the plugin. */
  export type Mounted = {
    drawn: () => Promise<RenderElement>
    find: (query: ElementQuery) => Promise<FoundElement | undefined>
    /** Presses the Button keyed `key`; resolves once its handler's work has settled. */
    press: (target: { key: string }) => Promise<unknown>
    unmount: () => Promise<void>
  }

  /** What `$.ui.mount` takes: the plugin, the surface (never defaulted), the component. */
  export type MountTarget = {
    [C in keyof RenderPropsOf]: {
      plugin: string
      surface: RenderSurface
      component: C
      props: RenderPropsOf[C]
      requestId?: string
      viewport?: RenderViewport
    }
  }[keyof RenderPropsOf]

  /** The engine a test drives: events raised as a session raises them, over every plugin. */
  export type Engine = {
    command: { run: (input: CommandRunInput) => Promise<CommandRunResult> }
    session: { start: (input: SessionStartInput) => Promise<{ cwd: string }> }
    turn: { complete: (input: TurnCompleteInput) => Promise<{ text: string }> }
    tool: { call: (input: ToolCallInput) => Promise<ToolCallResult> }
    ui: { mount: (target: MountTarget) => Promise<Mounted> }
  }

  /** A clock in memory that moves only when the test moves it. */
  export type MockClock = {
    now: () => number
    /** Moves the clock on, resolving each wait due on the way, then lets work settle. */
    advance: (ms: number) => Promise<void>
    /** Lets what is under way run as far as it can without moving the clock. */
    settle: () => Promise<void>
  }

  /** What the session's conversations had appended while the test ran. */
  export type MockSession = { appended: () => readonly unknown[] }

  export const mock: {
    clock: (on: On, options?: { now?: number }) => MockClock
    session: (on: On) => MockSession
  }

  export type Matchers = {
    toBe: (expected: unknown) => void
    toEqual: (expected: unknown) => void
    toBeLessThanOrEqual: (expected: number) => void
  }

  /** One test; its `on` hooks sit beneath the plugin under test. */
  export const test: (name: string, body: ($: Engine, on: On) => unknown) => void

  export const expect: (actual: unknown) => Matchers & { not: Matchers }
}
