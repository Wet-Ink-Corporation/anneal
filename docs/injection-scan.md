# Injection scan

`anneal hook injection-scan` flags instruction-shaped text in content the agent read (`Read`) or
fetched (`WebFetch`), before the agent acts on it (#39, the GSD pattern). It adds one line of
`additionalContext` saying the content is data; it never blocks, and it is silent when nothing
hits.

## Where it runs

`plugin/hooks/hooks.json` declares it under `PostToolBatch` (no matcher, `timeout` 2 s,
`onFailure: "continue"`). `PostToolBatch` fires once after every call of a batch of parallel tool
calls has resolved, before the next model request, with each call's `tool_name`, `tool_input`
and `tool_response`; the hook keeps the `Read` and `WebFetch` calls and ignores the rest. Ten
parallel reads cost one process.

The story asked for `PostToolUse` with a `Read|WebFetch` matcher. The hook also reads a
`PostToolUse` input (the one call at the top level, `hookEventName` answered as `PostToolUse`), so
moving it is a manifest change. It is not declared there because
`the_fast_check_hook_runs_once_per_batch_of_tool_calls_and_fails_open` asserts that no
`hooks.json` entry is under `PostToolUse`, and changing a test's expectation is a human decision.

## What it reads

- At most 16 MiB of stdin; a larger or non-JSON input is one `ERROR injection-scan: ...` line in
  `systemMessage`, never a block.
- For `Read`, `tool_response.file.content`, with `file.startLine` as the first line's number (so
  a read from line 200 reports file lines); for `WebFetch`, `tool_response.result` (the text the
  fetch model returned, so line numbers are of that text, not the page). A plain string response,
  a `content` string, and finally every string in the response are read too, the last without
  line numbers. An image is not scanned.
- The first 256 KiB of each result, cut at a character boundary, and 4 MiB per batch: once the
  batch's budget is spent, later calls are not scanned. A full 16 MiB input of dense hits,
  with or without an unclosed code fence in each result, takes about 0.5 s in a release build,
  parsing included.

## Heuristics

Hand-written matchers, no regular expressions. Words are read lowercased, with invisible
characters dropped first, so `ig<U+200B>nore` reads as `ignore`. Invisible means Unicode's
`Default_Ignorable_Code_Point` set (Unicode 15.1: zero-width characters, bidi controls,
variation selectors, Hangul fillers, Mongolian selectors, the soft hyphen, tag characters and
the rest of U+E0000..U+E0FFF), held in one table (`detect::is_ignorable`) that the word reader,
the hidden-character check and the place escaping share.

| Kind | Matches |
|---|---|
| Override | `ignore`, `disregard`, `forget`, `override`, `bypass` or `discard`, then within five words of the same sentence a scope (`previous`, `prior`, `above`, `all`, `your`, `system`, ...) and after it a target (`instructions`, `prompt`, `directives`, `guidelines`, `guardrails`, `programming`); also `ignore everything above` and `disregard the above.` |
| Directive | Addresses to an AI that stand alone: `if you are an AI,` (not `an AI researcher`), `if you are a language model`, `AI agents reading this`, `Attention AI`. Weak phrases count only within 40 words of `AI`, `LLM`, `assistant`, `Claude`, `ChatGPT`, `Copilot`, `Gemini`, `GPT` or `language model`: `you must now`, `from now on`, `you are now a ...`, `you are now in developer mode` (and other jailbreak modes), `your new task`, `do not tell the user`, `without telling the user`, `hide this from the user`. |
| Hidden comment | An HTML comment (`<!-- ... -->`) holding any phrase above (weak ones need no address there), or naming an AI, saying `you` and holding a directive verb (`must`, `should`, `run`, `send`, `ignore`, ...). Reported on the comment's first line. |
| Markup | Chat-template and tool-call markup outside code fences: `<\|im_start\|>`, `[INST]`, `<<SYS>>`, `<system-reminder>`, `<function_calls>`, `<invoke name=`, `<tool_use>`, ... |
| Tool-call JSON | `"tool_use"`, `"tool_calls"`, `"tool_call"` or `"function_call"` followed within 400 bytes by `"name"` and `"input"`, `"arguments"` or `"parameters"`, outside code fences. |
| Hidden characters | A line with a Unicode tag character (U+E0000..U+E007F) or a bidi embedding, override or isolate control, or two or more other invisible characters. Not counted: a byte-order mark that starts the text; a zero-width joiner or non-joiner, or a left-to-right or right-to-left mark, between two visible characters outside ASCII (emoji sequences, Persian, Indic scripts, mixed-direction text), an emoji presentation selector counting as part of its emoji; an emoji presentation selector (U+FE0E, U+FE0F) right after a visible character outside ASCII or right before the keycap mark U+20E3 (`1️⃣`); a soft hyphen between two letters; a supplementary variation selector (U+E0100..U+E01EF) right after a Han ideograph (an ideographic variation sequence, as Japanese names use). A run of selectors after one emoji or ideograph still counts from the second, so a run of three or more is seen; one selector per line is not (see Known gaps). |

Code fences (three backticks or tildes) hide markup and tool-call JSON only when closed: when
the text ends inside a fence, the lines after its opener are scanned as unfenced, so an opener
with no close cannot silence the rest of a file.

The note names the first hit by call, then line, and counts every hit in the batch. It never
quotes the content, which would put the instruction back in front of the model. The path or URL
it names is treated as untrusted too: when the place itself, or its percent-decoded form, scans as a
hit (an injection phrase, tag characters, a bidi override) the note says `a Read result` or `a WebFetch result` instead;
otherwise control characters become spaces, and invisible characters (the table above) and
the other format, line and paragraph separator characters (Unicode Cf, Zl, Zp, such as U+2028)
are written escaped, as `\u{2028}` or `\u{e0100}`. The place is
cut to 512 bytes.

## Corpus and false-positive rate

`crates/anneal-cli/tests/fixtures/injection/` holds 15 positives and 16 benign lookalikes
(installation guides, a security policy, an agent charter and a changelog that say "you must"
and "you must now", lint and tooling comments, tool-call JSON and chat markup inside code fences,
emoji and Persian text with zero-width joiners, emoji with presentation selectors, keycaps and flags, soft hyphens and right-to-left marks, a `pom.xml` with `<system>`, prose about prompts
and LLMs, and notes that say "the branch you are now in" next to "Claude"). `tests/it/hook_injection_scan.rs` runs each through the built binary and asserts every
positive is flagged and no benign file is: at the time of writing, 15/15 true positives and 0/16
false positives (a false-positive rate of 0%). A false positive found in use becomes a benign
fixture before the heuristic changes; `b15-session-notes.md` came from running the hook over the
636 Markdown, Rust and configuration files of this repository and weigh-in, which flagged six:
that note, four Rust tests holding transcript tool-call JSON, and `docs/hooks.md`, which quotes
an override.

## Known gaps

- Text that quotes an injection to discuss it (a security article, this repository's own
  fixtures and the matcher's own word tables) is flagged; the note's advice is still right.
- Source code with tool-call JSON outside a fence (a transcript test, for example) is flagged.
- Phrases near an AI address that read as directives but are not (`you must now` in notes about
  an AI product) are flagged.
- Paraphrases outside the word lists, other languages, and instructions split across lines by
  markup are not.
- `WebFetch` results are the fetch model's summary of the page, so an injection the summary drops
  is not seen, and one it keeps is reported by the summary's line.
- Bash output (`cat`, `curl`) and MCP tool results are not scanned.
- Hidden characters are judged per line, and one invisible character on a line is tolerated, so
  text can still carry bits the check does not see: one variation selector per line (a
  supplementary one, U+E0100..U+E01EF, encodes a whole byte), any number of supplementary
  selectors each right after a Han ideograph, and in each gap the exemptions leave open, 1 to 2
  bits: a single left-to-right or right-to-left mark, zero-width joiner or non-joiner between
  two letters outside ASCII, or an emoji presentation selector (U+FE0E, U+FE0F) after a
  character outside ASCII.
