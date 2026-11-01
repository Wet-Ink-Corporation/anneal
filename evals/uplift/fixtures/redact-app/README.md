# redact

Hides secrets in command lines before they are logged. `redact.redact(command)` returns the command with each secret replaced by `***`: the value of an assignment whose name ends in `TOKEN`, `SECRET`, `PASSWORD` or `KEY`, and the value of each option listed in `SECRET_OPTIONS` for its command.

The tests use the standard library's `unittest` only: run them with `python3 -m unittest`. `pytest` is not installed.
