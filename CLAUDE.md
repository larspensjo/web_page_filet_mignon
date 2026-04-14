@Agents.md

## Planning & Documentation
When creating or saving plan documents, always save them to the `docs/plans/` folder unless explicitly told otherwise.

## Skills
For research questions that should be answered from the local harvested article corpus, use the project skill in `.claude/skills/harvester-mcp-research/` and prefer the `harvester-mcp` MCP server over general model knowledge.
For those corpus-research tasks, do not inspect `output/*.md` directly with `Search`, `Read`, shell commands, or grep unless the MCP tools fail or prove insufficient. `harvester-mcp` is primarily a tools server, so a `listMcpResources` result of “no resources” is not a valid reason to avoid using its tools.
