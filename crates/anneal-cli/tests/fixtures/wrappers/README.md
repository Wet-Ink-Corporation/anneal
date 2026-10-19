# Wrapper probes

`probes.txt` records what each wrapper the guard unwraps actually runs, for the forms its
option grammar has to get right (#121): short-option clusters, value letters, getopt's `::`
optional letters, long options and their abbreviations, chrt's optional priority, and the last
`-c` of `su` and `script`. Each row is the word that ran (`W1` to `W9`), or `-` when nothing did,
a tab, then the command.

`probe.sh` runs every row on the binaries installed where it runs, with each `W<n>` replaced by
a recorder that only writes its own name: no row runs anything else, and nothing is deleted or
changed. Its header line names the versions it ran. `probe.sh --check` exits 1 when an
observation differs from the table, and `probe.sh > probes.txt.new` regenerates it (review the
diff before replacing the table). Run it as root in a container: `sudo` and `su` need it.

`denylist::tests::wrapper_probes` holds the guard to the table: for every row that ran, the
walk (`denylist::wrappers::unwrap`) of the row's wrapper must make that word the program of a
simple command, in itself or in the shell text it runs, or find a script known only at run
time (a process substitution, which the guard denies). A `-` row asserts nothing (the binary
refused the command or its policy did, so any reading is safe). doas and the BSD `env` and
`xargs`, read from their sources, are in `denylist::tests::wrapper_options`.
