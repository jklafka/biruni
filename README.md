# Biruni

[![CI](https://github.com/jklafka/biruni/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/jklafka/biruni/actions/workflows/ci.yml?query=branch%3Amain)
[![Deploy](https://github.com/jklafka/biruni/actions/workflows/deploy.yml/badge.svg?branch=main)](https://github.com/jklafka/biruni/actions/workflows/deploy.yml?query=branch%3Amain)

Biruni is a web app for exploring maps, written in Rust and compiled to WebAssembly. It is named after al-Bīrūnī (973–c. 1050), who measured the radius of the Earth, wrote on astronomy, and described the peoples of India.

**Live:** <https://biruni.jklafka.cloud>

## What you can do

- **Explore the Earth** on a 3D globe, or unroll it into a flat map (equirectangular, Mercator, Robinson or Mollweide). Drag to rotate, scroll or pinch to zoom from orbit down to about 400 km.
- **Switch layers:** political, monochrome, satellite, terrain, night lights, population density, and country statistics (population, GDP per capita, income group).
- **Turn on overlays:** day and night, the world's languages (by family, endangerment or grammar), traditional societies, historical borders from 3400 BCE to today (optionally by language family), and deep time, which moves the continents back 1000 million years.
- **Look up:** the sky as seen from any place and time, with stars, constellations, the Sun, Moon and planets, and optional sky-survey photographs.
- **Leave the Earth:** a solar system view with the planets to scale, their real surfaces, and spacecraft, asteroids and comets from NASA JPL.
- **Search, measure and share:** find places and languages, measure geodesic distances, click a country for facts, and share a link to exactly what you see.

## Getting around

| To | Do this |
| --- | --- |
| Move | drag, or the arrow keys |
| Zoom | scroll, pinch, or `+` / `-` |
| Fly to a place | double-click it, or search with `/` |
| Change the map layer | the layer picker (top right), or `1`–`9` |
| Overlays (day and night, history, languages, ...) | the layers button at the bottom right |
| Earth, sky or solar system | the switch beside the title, or `V` |
| Start over | ⟲, or `R` |

Every control, with touch and keyboard paths, is in [docs/controls.md](docs/controls.md), along with how each view and tool behaves.

## Running it locally

You need Rust via [rustup](https://rustup.rs) with `rustup target add wasm32-unknown-unknown`, [Trunk](https://trunkrs.dev) and Python 3.

```sh
trunk serve --open      # dev server with live reload at http://127.0.0.1:8080
cargo test              # unit tests
```

## Documentation

| Read | For |
| --- | --- |
| [docs/controls.md](docs/controls.md) | Every control, the views and overlays, and the URL format |
| [docs/development.md](docs/development.md) | Building, testing (including the visual regression tests) and debug aids |
| [docs/architecture.md](docs/architecture.md) | How it works: the camera, rendering, layers, labels, search |
| [docs/data.md](docs/data.md) | Where each dataset comes from and how it is converted and served |
| [docs/deployment.md](docs/deployment.md) | Cloudflare Pages, the data on R2, the weekly JPL refresh, tile packs |
| [docs/tile-streaming.md](docs/tile-streaming.md), [docs/historical-data.md](docs/historical-data.md) | Design notes on detail tiles and the historical data format |
| [AGENTS.md](AGENTS.md) | Contribution rules and conventions, for people and coding agents |
| [DATA_LICENSES.md](DATA_LICENSES.md) | The license and required credit of every dataset |

## Data and credits

Biruni is built on open data: [Natural Earth](https://www.naturalearthdata.com/) (public domain) for the base map, NASA imagery (Blue Marble, Black Marble, planetary maps and JPL Horizons), the JRC's GHS-POP population grid, Glottolog and Grambank for languages, D-PLACE for societies, Cliopatria and the Seshat Global History Databank for history, the Merdith et al. plate model for deep time, and the HYG catalog, Stellarium and the Digitized Sky Survey for the sky. The app's "Data credits" link lists the sources of what is on screen. Some data is non-commercial (CC BY-NC), so Biruni stays a non-commercial project; see [DATA_LICENSES.md](DATA_LICENSES.md) and [docs/data.md](docs/data.md).

## Contributing

Every change goes through a pull request, and CI (format, clippy, tests, shader validation, data checks, a release build and visual regression tests) must pass before it merges. Start with [AGENTS.md](AGENTS.md) and [docs/development.md](docs/development.md).
