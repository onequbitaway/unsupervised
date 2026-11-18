# The viewer

This is the 2D viewer of Unsupervised. It shows the world live: the country
as a population heat map, a city with its districts, and a district at
street level with its residents. The server serves the built viewer at `/`.

The viewer speaks the viewer feed of `docs/design/protocol.md`, section 5.
It downloads no art: tiles, figures and markers are painted from code.

## Commands

Run these commands in `web/`.

| Command | What it does |
| --- | --- |
| `npm ci` | Install the packages of the lockfile |
| `npm run build` | Check the types and write `dist/` |
| `npm test` | Run the unit tests. They need no browser |
| `npm run dev` | Serve the viewer with reload. Set `UNSUP_SERVER` to the address of a running server (default `http://127.0.0.1:8080`) |
| `npm run screenshot -- --url <server>` | Save a PNG of the viewer |
| `npm run perf -- --url <server>` | Measure the time between frames |
| `npm run populate -- --url <server>` | Put residents into a development server and make them walk |

`screenshot` and `perf` drive the Google Chrome of this machine through
`playwright-core`. They download no browser. Set `CHROME_PATH` when Chrome is
not at `/Applications/Google Chrome.app`. Each script lists its options at
the top of its file in `scripts/`.

`populate` uses the development shortcuts of the server (`/v1/dev/residents`
and the resident id as the token). When real sign-up replaces them, change
the three functions `createResident`, `whereIs` and `act`.

## Options in the address

| Option | Meaning |
| --- | --- |
| `?watch=<resident id>` | Follow one resident, also into other districts and cities. Share this link |
| `?broadcast=1` | Hide the controls, for a video stream |
| `?view=country`, `?view=city`, `?view=street` | The zoom level at the start |
| `?zone=<zone id>` | The district to look at at the start |

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
| `src/state` | Pure functions: the reducer of the feed, movement between ticks, the camera, the ticker, clean text |
| `src/render` | The picture (PixiJS): land, tiles, sites, residents, names, speech bubbles |
| `src/ui` | The page around the picture: head, controls, ticker, resident panel |
| `src/app` | The parts that connect the others: camera rig, map loader, news desk, the viewer |
| `src/config.ts` | Every number that a designer could want to tune |
| `scripts` | Screenshot, measurement and the development script |

## Rules

- Text from residents is untrusted. Clean it with `cleanText` and show it
  only as text. Never give it to `innerHTML`.
- Keep the state code pure and test it. The tests of `src/ui` use
  `happy-dom`; all other tests run in plain Node.
- Put every tunable number in `src/config.ts`.

## A world to work with

The game modules are built in parallel with the viewer. Until they are
ready, the example `showcase` of the spectator crate gives the viewer a
town with streets, sites, walking residents, speech and events. It is a
tool, not the game.

```sh
npm run build
cd ..
cargo run --release -p unsup-spectator --example showcase -- --residents 400
```

Then open `http://127.0.0.1:8090/`. With `--residents 5000 --cities 1
--districts 1` the showcase puts 5,000 residents into one district; use it
with `npm run perf` to check that the viewer stays smooth.
