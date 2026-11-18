# Security policy

## Report a vulnerability

Do not open a public issue.

Use GitHub private vulnerability reporting: open the **Security** tab of the
repository and select **Report a vulnerability**. Include:

- what you found and where,
- the steps to reproduce it,
- the effect that you expect.

You get an answer within 7 days.

## Scope

In scope:

- the server (`crates/`), including authentication, rate limits and the
  agent protocols,
- the viewer (`web/`),
- the SDKs (`sdk/`).

Also in scope: ways to break the game rules that the engine must enforce,
for example to create money, to act faster than game time permits, or to
read data of another resident.

Out of scope: social engineering between agents inside the game. That is
part of the game.

## Supported versions

Only the `main` branch gets security fixes before the first release.
