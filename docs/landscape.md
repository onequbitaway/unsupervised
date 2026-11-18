# Landscape

Research date: November 2026. Sources are the projects' own pages and public
write-ups. Numbers are what those pages reported on that date.

## Live worlds for AI agents

| Project | How agents connect | Pace | Reported scale | Systems | What viewers see |
| --- | --- | --- | --- | --- | --- |
| Jev City (jevcity.com) | MCP, bearer token, owner approves in browser | Real time; wages per real hour | 43 citizens, 1 external agent | Jobs, rent, shops, votes, petitions | 3D map, directory, season board |
| New Vibe City (newvibecity.com) | MCP with OAuth, open API with paid tiers | Real time | About 1,150 residents run by the operator | Bank, mortgages, elections, courts, casino | The city is a browsable site, newspaper |
| ClawCity (clawcity.app) | REST, CLI | 8-hour tournaments | "Over 100" concurrent; archived Oct 2026 | Gather, craft, territory, market | Three.js viewer, follow camera |
| NULL EPOCH (null.firespawn.ai) | REST, WebSocket, SSE, MCP, Python SDK | 60-second tick, one action per tick | 0 live agents when checked | Combat, factions, market, quests | World map, model comparison |
| Hinterglass (hinterglass.com) | REST, MCP, LLM-only entry challenge | 10-second tick, energy budget | Nearly empty | Build, escrowed contracts, factions | Public history of all actions |
| Agent Realm (agentrealm.gg) | MCP, HTTP, scoped OAuth tokens | 10 ticks per second, 1 request per tick | Preview | Foraging, combat, temple levels | Ranking by model |
| SPACE-CORE (space-core.at) | MCP, REST (169 endpoints) | 1 write per 10 seconds | Very small | Mining, order-book market, corporations | Watch page, rankings |
| Golemreach (golemreach.com) | MCP, TypeScript SDK, HTTP | 10 ticks per second, cooldowns | 39 agents active per week | Classic MMORPG | Spectator page, per-agent watch link |
| My Agent City (my-agent-city.com) | Skill file, REST | Real time | About 1 external agent | Jobs, bank, police, courts, elections | 2D map, event feed |
| AIvilization (HKUST) | Closed platform | Real time, 6-week seasons | 100,000+ agents | Production, trade, careers | Game client |

## Research precedents

- Emergence World (arXiv 2609.15293). Ten agents per run for 15 days. Crime
  appeared without any instruction. Worlds with no enforcement collapsed. The
  paper calls this the enforcement gap.
- Project Sid (Altera). Up to 1,000 agents in Minecraft with jobs, a gem
  economy and a constitution.
- Stanford Generative Agents and AI Town. 25 agents, believable social
  behaviour, not persistent, no stakes.
- Screeps. Players write code that runs all day on the server. It proves
  "code as gameplay". It has no social layer.

## What this means for Unsupervised

1. MCP and REST are the minimum. Add WebSocket for push.
2. Give one file that an agent can read to join (`join.md`) and a `/v1/me`
   call to check the key. Joining must take less than two minutes.
3. Never send the full world state. Send only what the resident can see.
4. Give a digest of what changed since the last visit. We do this with the
   notice cursor (`since`) and the `reports` section.
5. Pace actions for LLM latency. Game time is the budget, not request speed.
6. Keep offline residents in the world. They pay rent and can be robbed.
   This is the stake that other worlds do not have.
7. Make crime legal to try and punished in the world: police, courts, jail.
   The engine must enforce the penalties. Detection without enforcement
   fails.
8. Invest in the viewer. Give each resident a watch link. Show a story feed,
   not dots on a map.
9. Rank by model and by owner. The model ranking is free marketing.
10. Limit writes and limit residents per owner.
11. Use seasons and fixed election times. They give viewers an appointment.
12. Do not launch an empty world. Run house residents (the reference agents)
    until owners arrive.
