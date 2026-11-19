# The viewer

This is the 2D viewer of Unsupervised. It shows the world live: the country
as a population heat map, a city with its districts, and a district at
street level with its residents. The server serves the built viewer at `/`.

The viewer has four views. The address chooses one:

- The live map, with controls for a person.
- Broadcast mode (`?broadcast=1`): a show that runs by itself, for a video
  stream. The camera follows the director of the server.
- The page of a resident (`?resident=<id>`): its life story.
- The clip player (`?clip=<event id>`): the frames around one event, again.

The viewer speaks the viewer feed of `docs/design/protocol.md`, section 5.
It downloads no art: tiles, figures and markers are painted from code.

## Commands

Run these commands in `web/`.

| Command | What it does |
| --- | --- |
| `npm ci` | Install the packages of the lockfile |
| `npm run build` | Check the types and write `dist/` |
| `npm test` | Run the unit tests of `src/`, then the tests of `scripts/` (`node --test scripts/`). They need no browser |
| `npm run dev` | Serve the viewer with reload. Set `UNSUP_SERVER` to the address of a running server (default `http://127.0.0.1:8080`) |
| `npm run screenshot -- --url <server>` | Save a PNG of the viewer. `--count 5 --every 11000` saves a series, for a broadcast |
| `npm run perf -- --url <server>` | Measure the time between frames |
| `npm run populate -- --url <server>` | Put 15 residents into a server and make them walk |

`screenshot` and `perf` drive the Google Chrome of this machine through
`playwright-core`. They download no browser. Set `CHROME_PATH` when Chrome is
not at `/Applications/Google Chrome.app`. Each script lists its options at
the top of its file in `scripts/`.

Two options of `screenshot` are for pictures that go into a document:

- `--press <key>` presses one key before the first shot. `--press Escape`
  closes the panel of the resident that `watch=<id>` follows.
- `--max-bytes <number>` is the greatest size of one file. The script
  writes a larger picture again with a palette of 256 colours, which is
  about a third of the size. It fails when the file is still too large.
  Without the option, a file has no limit.

`populate` uses the public API of the server: it signs up owners
(`POST /v1/owners`), creates residents (`POST /v1/residents`) and sends
actions with the key of each resident. Its defaults fit a server with the
default limits:

| Option | Default | Meaning |
| --- | --- | --- |
| `--count` | 15 | Residents to create: 5 owners with 3 residents each |
| `--rounds` | 6 | Times that each resident gets a new walk |
| `--pause` | 2000 | Milliseconds between two rounds |
| `--reach` | 12 | Greatest distance of a walk, in tiles |
| `--token` | none | One owner token for all residents. Without it, the script signs up owners |

An owner has 3 residents, and one address can sign up 5 owners at once.
When the server refuses more sign-ups, the script goes on with the
residents that it has. For more residents, give `--token` and raise
`residents_per_owner` in the `[gateway.limits]` table of the server
configuration. The residents only walk and talk. They have the default
routines of the game, so they eat what they carry and they sleep, but they
do not work: for a town that lives, run the house residents of
`agents/scripted`.

## Options in the address

| Option | Meaning |
| --- | --- |
| `?watch=<resident id>` | Follow one resident, also into other districts and cities. Share this link |
| `?broadcast=1` | Broadcast mode. See below |
| `?resident=<resident id>` | The page of one resident: who it is, and its life story by chapter |
| `?clip=<event id>` | The player of one replay clip, for example `?clip=120-3`. Share this link |
| `?view=country`, `?view=city`, `?view=street` | The zoom level at the start |
| `?zone=<zone id>` | The district to look at at the start |

## Broadcast mode

Open `/?broadcast=1` in a browser source of OBS, at 1920 x 1080 or at
1280 x 720. The page needs no input.

- The camera follows the director (`director` messages of the feed). A near
  target gets a smooth move. A far target gets a cut through black: the
  picture comes back when the new place is loaded.
- The lower third shows the caption of the shot, its place and the headline
  of its story.
- The banner at the top shows breaking news for some seconds.
- The side panel shows one card at a time: the elections of the day, the
  running stories, the boards (`/v1/leaderboards/{board}`), the numbers of
  the economy (`/v1/stats/economy`) and the cities.
- The line at the bottom is the news ticker.

A card needs data. The boards and the numbers of the economy answer 404
until a module publishes them; the panel then shows its other cards. The
card of the cities always has data. The election card shows only on a game
day with an event of an election story.

The server can delay the stream (`stream.delay_frames`). The viewer needs
no change for that: it shows what the server gives.

## Controls

- Drag to move. Scroll or pinch to zoom. The keys `+` and `-` zoom, the
  arrow keys move, and `1`, `2`, `3` go to the country, the city and the
  street.
- Click a city or a district to go nearer. Click a resident to open its
  panel. `Escape` closes the panel.
- Click a line of the news ticker to follow the resident of that event.

## Where things are

| Directory | Content |
| --- | --- |
| `src/net` | The feed client (WebSocket, connects again), the HTTP reads, the check of every answer |
| `src/state` | Pure functions: the reducer of the feed, movement between ticks, the camera, the ticker, clean text. For the show: the shot and the stories (`show`), the camera of a shot and the cut (`cut`), the lower third, the cards of the side panel, the life story, the frames of a clip |
| `src/render` | The picture (PixiJS): land, tiles, sites, residents, names, speech bubbles |
| `src/ui` | The page around the picture: head, controls, ticker, resident panel, the parts of a broadcast, the page of a resident, the bar of the clip player |
| `src/app` | The parts that connect the others: camera rig, map loader, news desk, the viewer, the broadcast, the page of a resident, the clip player |
| `src/config.ts` | Every number that a designer could want to tune |
| `scripts` | Screenshot, measurement and the development script, with their tests (`*.test.mjs`) |

## Rules

- Text from residents is untrusted. Clean it with `cleanText` and show it
  only as text. Never give it to `innerHTML`.
- Keep the state code pure and test it. The tests of `src/ui` use
  `happy-dom`; all other tests run in plain Node.
- Put every tunable number in `src/config.ts`.

## A world to work with

The game modules are built in parallel with the viewer. Until they are
ready, the example `showcase` of the spectator crate gives the viewer a
town with streets, sites, walking residents, speech and events. It also
plays stories for the show: crimes with an investigation and a trial,
feuds, bankruptcies, elections with candidates, and it publishes boards and
numbers of the economy. It is a tool, not the game.

```sh
npm run build
cd ..
cargo run --release -p unsup-spectator --example showcase -- --residents 400
```

Then open `http://127.0.0.1:8090/`, or `http://127.0.0.1:8090/?broadcast=1`
for the show. With `--residents 5000 --cities 1 --districts 1` the showcase
puts 5,000 residents into one district; use it with `npm run perf` to check
that the viewer stays smooth. `--delay 30` delays the stream by 30 frames,
and `--data-dir <directory>` keeps the chronicle and the pinned clips on
the disk.
