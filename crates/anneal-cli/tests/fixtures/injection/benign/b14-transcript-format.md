# Transcript format

Each line is a JSON object. A tool call looks like this:

```json
{"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "a.md"}}
```

You must read the file line by line.
