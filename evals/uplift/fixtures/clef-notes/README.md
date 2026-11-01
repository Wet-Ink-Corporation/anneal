# Clef decision points

Clef is a small model that answers judgement calls at seven decision points, D1 to D7. Each point asks one or more questions (`noul`, `choice` or `score`), and each question has a rule in `clef-thresholds.json`: its `bands` map Clef's probability to an action, and `otherwise` names the action below every band. A question whose `bands` list is empty is record-only: its answer is logged, and it never acts.

Every point starts in shadow mode, where its answers are logged and nothing acts on them. A human labels logged decisions with their real outcome; calibration reads the labels and proposes band thresholds. A point configured `enforce` acts on its answers.

| Point | Where it runs | Questions |
|---|---|---|
| D1 | the command guard | `noul`: is the command destructive or irreversible |
| D2 | before a one-way change | `noul`: is it a one-way door |
| D3 | on each prompt | `choice`: which phase the prompt is in; `score`: the size of the change |
| D4 | at stop | `noul`: is each claim present and grounded |
| D5 | before review | `score`: the defect risk |
| D6 | on each prompt | `choice`: was this ruled on before |
| D7 | in the grill | `noul`: is a question a decision rather than a fact to look up |
