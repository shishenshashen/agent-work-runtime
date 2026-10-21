# Cursor integration examples

[L1 Cursor note](../../docs/dev/integrations/cursor.md) records merge paths.
Session lifecycle stays on the [L0 workflow](../../docs/dev/integrations/session-workflow.md).

Published AWR 0.6.0 and a binary built from this repository expose the grouped
catalog this template expects. The 0.4.0 package does not. 0.5.1 already grouped
`awr_query`; install 0.6.0 so schema 14 and `session start --client/--conversation`
match this tree.

`mcp.json.example` is a stdio template with Cursor's documented fields (`type`,
`command`, `args`). Merge it into `.cursor/mcp.json` or `~/.cursor/mcp.json`.
Do not commit a project file that points at a machine-local AWR root.
