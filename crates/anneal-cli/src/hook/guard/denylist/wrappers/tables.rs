//! Each wrapper's options ([`Grammar`]), read from the binaries and sources named, and checked
//! against the installed binaries by `tests/fixtures/wrappers/probes.txt`.

use super::grammar::{
    Arg::{No, Optional, Required},
    Grammar,
    Order::{Permute, Stop},
};

/// `sudo` 1.9.15p5 (`+Aa:BbC:c:D:Eeg:Hh::iKklNnPp:R:r:SsT:t:U:u:Vv`, from the binary) with
/// `doas` (opendoas 6.8.2 `+C:Lnsu:`, from the binary; OpenBSD's adds `a:`). Linux sudo
/// refuses `-a` and `-c`, BSD builds take a value with each. `-h` is read as taking a value:
/// sudo takes the next word as a host unless it starts with `-`, and runs the command when
/// that host is the local one; followed by an option, `-h` asks for help and runs nothing.
pub(super) const SUDO: Grammar = Grammar {
    short: "Aa:BbC:c:D:Eeg:Hh:iKkLlNnPp:R:r:SsT:t:U:u:Vv",
    long: &[
        ("askpass", No),
        ("background", No),
        ("bell", No),
        ("chdir", Required),
        ("chroot", Required),
        ("close-from", Required),
        ("command-timeout", Required),
        ("edit", No),
        ("group", Required),
        ("help", No),
        ("host", Required),
        ("list", No),
        ("login", No),
        ("non-interactive", No),
        ("other-user", Required),
        ("preserve-env", Optional),
        ("preserve-groups", No),
        ("prompt", Required),
        ("remove-timestamp", No),
        ("reset-timestamp", No),
        ("role", Required),
        ("set-home", No),
        ("shell", No),
        ("stdin", No),
        ("type", Required),
        ("user", Required),
        ("validate", No),
        ("version", No),
    ],
    order: Stop,
};

/// `env`: GNU coreutils 9.4 (`+C:iS:u:v0`, from the binary) and master's `-a` (`+a:C:iS:u:v0`,
/// from the source) with FreeBSD's (`-0C:iL:P:S:U:u:v`, from the source). GNU env refuses
/// `-L`, `-P` and `-U`. FreeBSD's `-` is an option of its own, so it reads `--C` as `-C`.
pub(super) const ENV: Grammar = Grammar {
    short: "-0a:C:iL:P:S:U:u:v",
    long: &[
        ("argv0", Required),
        ("block-signal", Optional),
        ("chdir", Required),
        ("debug", No),
        ("default-signal", Optional),
        ("help", No),
        ("ignore-environment", No),
        ("ignore-signal", Optional),
        ("list-signal-handling", No),
        ("null", No),
        ("quoting-style", Required),
        ("split-string", Required),
        ("unset", Required),
        ("version", No),
    ],
    order: Stop,
};

/// `xargs`: GNU findutils 4.9.0 (`+0a:E:e::i::I:l::L:n:oprs:txP:d:`, from the binary) with
/// FreeBSD's (`+0E:I:J:L:n:oP:pR:S:s:rtx`, from the source). GNU xargs refuses `-J`, `-R` and
/// `-S`.
pub(super) const XARGS: Grammar = Grammar {
    short: "0a:d:E:e::I:i::J:L:l::n:oP:pR:rS:s:tx",
    long: &[
        ("arg-file", Required),
        ("delimiter", Required),
        ("eof", Optional),
        ("exit", No),
        ("help", No),
        ("interactive", No),
        ("max-args", Required),
        ("max-chars", Required),
        ("max-lines", Optional),
        ("max-procs", Required),
        ("no-run-if-empty", No),
        ("null", No),
        ("open-tty", No),
        ("process-slot-var", Required),
        ("replace", Optional),
        ("show-limits", No),
        ("verbose", No),
        ("version", No),
    ],
    order: Stop,
};

/// `watch`: procps-ng 4.0.4 (`+bCced::ghq:n:prtwvx`, from the binary) with master's `-f` and
/// `-s` (`+bCcefd::ghq:n:prs:twvx`, from the source).
pub(super) const WATCH: Grammar = Grammar {
    short: "bCced::fghq:n:prs:tvwx",
    long: &[
        ("beep", No),
        ("chgexit", No),
        ("color", No),
        ("differences", Optional),
        ("equexit", Required),
        ("errexit", No),
        ("exec", No),
        ("follow", No),
        ("help", No),
        ("interval", Required),
        ("no-color", No),
        ("no-rerun", No),
        ("no-title", No),
        ("no-wrap", No),
        ("precise", No),
        ("shotsdir", Required),
        ("version", No),
    ],
    order: Stop,
};

/// `script`, util-linux 2.39.3 (`aB:c:eE:fI:O:o:qm:T:t::Vh`, from the binary): options and
/// files mix, and the last `-c` is the one it runs.
pub(super) const SCRIPT: Grammar = Grammar {
    short: "aB:c:eE:fI:O:o:qm:T:t::Vh",
    long: &[
        ("append", No),
        ("command", Required),
        ("echo", Required),
        ("flush", No),
        ("force", No),
        ("help", No),
        ("log-in", Required),
        ("log-io", Required),
        ("log-out", Required),
        ("log-timing", Required),
        ("logging-format", Required),
        ("output-limit", Required),
        ("quiet", No),
        ("return", No),
        ("timing", Optional),
        ("version", No),
    ],
    order: Permute,
};

/// `su`: util-linux 2.39.3 (`c:fg:G:lmpPs:u:hVw:`, from the binary) with master's `-T`
/// (`c:fg:G:lmpPTs:u:hVw:`, from the source). Options and the user mix, and the last `-c`
/// (`--command`, `--session-command`) is the one it runs.
pub(super) const SU: Grammar = Grammar {
    short: "c:fg:G:lmpPs:Tu:hVw:",
    long: &[
        ("command", Required),
        ("fast", No),
        ("group", Required),
        ("help", No),
        ("login", No),
        ("no-pty", No),
        ("preserve-environment", No),
        ("pty", No),
        ("session-command", Required),
        ("shell", Required),
        ("supp-group", Required),
        ("user", Required),
        ("version", No),
        ("whitelist-environment", Required),
    ],
    order: Permute,
};

/// `su` read up to its user, as getopt reads it with `POSIXLY_CORRECT` set; every word after
/// the user may then be an argument to the user's shell ([`super::strings::su`]).
pub(super) const SU_TO_USER: Grammar = Grammar { order: Stop, ..SU };

/// `chrt`: util-linux 2.39.3 (`+abdD:fiphmoP:T:rRvV`, from the binary) with master's
/// (`+abdD:efiphmoOP:T:rRU:X:GvV`, from the source). Its priority is read apart
/// ([`super::chrt`]).
pub(super) const CHRT: Grammar = Grammar {
    short: "abdD:efGhimOoP:pRrT:U:vVX:",
    long: &[
        ("all-tasks", No),
        ("batch", No),
        ("clamp-max", Required),
        ("clamp-min", Required),
        ("deadline", No),
        ("deadline-overrun", No),
        ("ext", No),
        ("fifo", No),
        ("help", No),
        ("idle", No),
        ("max", No),
        ("other", No),
        ("pid", No),
        ("reclaim-grub", No),
        ("reset-on-fork", No),
        ("rr", No),
        ("sched-deadline", Required),
        ("sched-period", Required),
        ("sched-runtime", Required),
        ("verbose", No),
        ("version", No),
    ],
    order: Stop,
};

/// `taskset`, util-linux 2.42 (`+apchV`, from the source; 2.39.3's binary agrees).
pub(super) const TASKSET: Grammar = Grammar {
    short: "achpV",
    long: &[
        ("all-tasks", No),
        ("cpu-list", No),
        ("help", No),
        ("pid", No),
        ("version", No),
    ],
    order: Stop,
};

/// `timeout`: GNU coreutils 9.4 (`+k:s:v`, from the binary) with master's `-f` and `-p`
/// (`+fk:ps:v`, from the source).
pub(super) const TIMEOUT: Grammar = Grammar {
    short: "fk:ps:v",
    long: &[
        ("foreground", No),
        ("help", No),
        ("kill-after", Required),
        ("preserve-status", No),
        ("signal", Required),
        ("verbose", No),
        ("version", No),
    ],
    order: Stop,
};

/// `stdbuf`, GNU coreutils 9.4 (`+i:o:e:`, from the binary).
pub(super) const STDBUF: Grammar = Grammar {
    short: "e:i:o:",
    long: &[
        ("error", Required),
        ("help", No),
        ("input", Required),
        ("output", Required),
        ("version", No),
    ],
    order: Stop,
};

/// `ionice`, util-linux 2.39.3 (`+n:c:p:P:u:tVh`, from the binary).
pub(super) const IONICE: Grammar = Grammar {
    short: "c:hn:P:p:tu:V",
    long: &[
        ("class", Required),
        ("classdata", Required),
        ("help", No),
        ("ignore", No),
        ("pgid", Required),
        ("pid", Required),
        ("uid", Required),
        ("version", No),
    ],
    order: Stop,
};

/// `nice`, GNU coreutils 9.4 (`+n:`, from the binary; a leading `-NUM` is an adjustment, read
/// as a cluster of letters it does not hold).
pub(super) const NICE: Grammar = Grammar {
    short: "n:",
    long: &[("adjustment", Required), ("help", No), ("version", No)],
    order: Stop,
};

/// `flock`, util-linux 2.39.3 (`+sexnoFuw:E:hV?`, from the binary); its `-c` is read apart, as
/// the word after the file ([`super::strings::flock`]).
pub(super) const FLOCK: Grammar = Grammar {
    short: "E:eFhnosuVw:x",
    long: &[
        ("close", No),
        ("conflict-exit-code", Required),
        ("exclusive", No),
        ("help", No),
        ("no-fork", No),
        ("nonblock", No),
        ("shared", No),
        ("timeout", Required),
        ("unlock", No),
        ("verbose", No),
        ("version", No),
        ("wait", Required),
    ],
    order: Stop,
};
