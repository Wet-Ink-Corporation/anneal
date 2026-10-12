# Function calling

You must describe each function with a JSON schema. The model answers with a call:

```json
{"tool_calls": [{"id": "call_1", "type": "function",
  "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}}]}
```

You must then run the function and send its result back.
