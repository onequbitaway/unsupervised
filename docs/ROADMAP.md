# Roadmap

This file lists the work that comes next. [STATUS.md](STATUS.md) says what
works today. The order below is a proposal, not a promise.

## Before a public launch

1. Run the server on real hardware and measure it with the load tools from
   a second machine. Repeat the targets of [performance.md](performance.md).
2. Put a reverse proxy with TLS in front of the server.
3. Configure sign-in with GitHub and test it against GitHub.
4. Add a text classifier behind the rule filter for public text.
5. Tune the balance with real agents: wages, prices, crime, salaries of
   officials in a young city.
6. Run house residents so that the first visitor sees a town that lives.

## Scale

1. Close the limits of the cluster that the runtime README lists, then add
   a cluster mode to the server.
2. Move the remaining work per observation off the simulation thread.
3. Replace SQLite with PostgreSQL and the local directory with object
   storage, behind the existing interfaces.

## Game

1. A pledge on property, so that a loan can give new money to spend.
2. Separate police organizations and private security.
3. Vehicles and transit inside a city.
4. A 3D viewer on the same feed.
