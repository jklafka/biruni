# Data licenses

Every dataset Biruni uses, or plans to use, is listed here with its license and the credit it requires. Check this file before writing a converter for a new dataset, and add the dataset here in the same PR that adds it. `README.md` keeps the user-facing credits.

This is a working record, not legal advice. Where a license is marked unverified, read the dataset's own license file for the exact version you download before you use it.

## Policy

| License family | Can we ship it? | What it requires |
| --- | --- | --- |
| Public domain, CC0, US government works | Yes | Nothing, but credit the source anyway |
| CC BY | Yes | Credit in the app (see below) and in `README.md`, link to the license, say if we changed the data |
| CC BY-SA, ODbL, GPL (as a data license) | Yes, with care | As CC BY, and the files we derive from it (our `.bin` or `.tsv`) must be offered under the same license. Keep share-alike data in its own files, so the license does not spread to other layers. It does not affect the app's code |
| Non-commercial (CC BY-NC, CC BY-NC-SA) | Yes, with in-app credit | The owner decided (2026-11-15) that Biruni is non-commercial. Using NC data makes that lasting: no ads, paid tiers or sponsorship while it is served. NC-SA data also follows the share-alike rule above |
| No-derivatives (CC BY-ND, CC BY-NC-ND) | No | We always convert data, which is a derivative |
| No license stated | No | Ask the authors, or find another source |

**Credit in the app:** CC BY and share-alike licenses require credit where the data is shown, not only in the repository. Before shipping the first such dataset, the app needs a credits panel that lists each visible layer's source and license.

## In use

A `[key: id]` tag in a dataset cell is the id that `data-sources.lock.json` entries use in their `license` field (`scripts/data_sources.py check` verifies it). Add a tag to any dataset that gets a lockfile entry.

| Dataset | Used for | License | Credit | Verified |
| --- | --- | --- | --- | --- |
| [Natural Earth](https://www.naturalearthdata.com/) 1:50m and 1:10m vectors [key: natural-earth] | Land, lakes, borders, countries, glaciers, ice shelves, places, labels, marine names | Public domain | "Made with Natural Earth" (requested, not required) | Yes, naturalearthdata.com terms |
| [NASA Blue Marble: Next Generation](https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-topography-bathymetry) (July 2004, with GEBCO bathymetry) [key: nasa-bmng] | Satellite layer | US government work, generally not subject to US copyright. NASA insignia and logos excluded (none used) | "NASA Earth Observatory" | Yes, NASA media usage guidelines |

## Candidates for the roadmap

| Dataset | Planned use | License | Status | Notes |
| --- | --- | --- | --- | --- |
| [HYG star database](https://codeberg.org/astronexus/hyg) v4.x | Night sky stars | CC BY-SA 4.0 (v4.0 and later; earlier versions CC BY-SA 2.5) | Usable with care | Share-alike: ship the star file under CC BY-SA 4.0 |
| [Gaia](https://www.cosmos.esa.int/web/gaia-users/license) DR3 (bright subset) | Night sky stars, later | Free to use with credit "ESA/Gaia/DPAC" | Usable | ESA's images and graphics are CC BY-SA 3.0 IGO; the catalog data is credit only. Recheck for DR4 |
| [Stellarium sky cultures](https://github.com/Stellarium/stellarium-skycultures), `western` | Constellation lines and names | Text and data CC BY-SA; illustrations Free Art License | Usable with care | Licensed per culture: some others are CC BY-NC-SA or CC BY-NC-ND. Check each culture's `description.md` |
| [JPL Horizons](https://ssd-api.jpl.nasa.gov/doc/horizons.html) | Spacecraft and small bodies, scheduled refresh | US government service; no published license or rate policy found | Usable | Credit "NASA/JPL Horizons". Keep requests modest and cache results; check the Horizons news page for service changes |
| [Glottolog](https://github.com/glottolog/glottolog-cldf) | Languages | CC BY 4.0 (glottolog-cldf) | Usable | |
| [Cliopatria](https://github.com/Seshat-Global-History-Databank/cliopatria) (Seshat) | Historical borders, 3400 BCE to 2024 | CC BY 4.0 (repository LICENSE) | Usable | Seshat's other datasets are CC BY-NC-SA; this one is not |
| [historical-basemaps](https://github.com/aourednik/historical-basemaps) | Historical borders | GPL-3.0 | Usable with care | Share-alike via GPL: ship derived files under GPL-3.0, separately from other layers. Prefer Cliopatria where it covers the same period |
| [CShapes 2.0](https://icr.ethz.ch/data/cshapes/) | Borders 1886 to 2019 | Non-commercial: CC BY-NC 4.0 per the ETH Research Collection, CC BY-NC-SA 4.0 per another catalog | Usable once verified | Confirm the exact license on the ICR download page. The R package's GPL covers its code, not the data |
| [D-PLACE](https://github.com/D-PLACE/dplace-cldf) | Ethnographic data | CC BY-NC 4.0 (repository LICENSE) | Usable | Non-commercial (allowed by the policy above) |
| [PALEOMAP](https://www.earthbyte.org/?p=3670) PaleoAtlas, PaleoDEMs and coastlines (Scotese; Kocsis and Scotese) | Paleogeography | Not found; described as free for research and publications | **Blocked**, unverified | Read the license on the Zenodo record for the exact version, or ask the authors |
