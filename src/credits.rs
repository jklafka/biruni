//! Data credits: the datasets behind what is on screen, with their license.
//!
//! Browser-free. `DATA_LICENSES.md` is the full record; this table holds the
//! part the app shows. A test checks that every dataset in that file's "In
//! use" table has an entry here.

use crate::view::Layer;

pub struct Credit {
    pub id: &'static str,
    pub name: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
    pub source_url: &'static str,
    /// What the data is used for, and the credit line it asks for.
    pub text: &'static str,
}

pub const NATURAL_EARTH: Credit = Credit {
    id: "natural-earth",
    name: "Natural Earth",
    license: "Public domain",
    license_url: "https://www.naturalearthdata.com/about/terms-of-use/",
    source_url: "https://www.naturalearthdata.com/",
    text: "Coastlines, borders, countries, places, labels, country statistics and the terrain relief. Made with Natural Earth.",
};

pub const BLUE_MARBLE: Credit = Credit {
    id: "nasa-bmng",
    name: "NASA Blue Marble: Next Generation",
    license: "US government work (NASA media guidelines)",
    license_url: "https://www.nasa.gov/nasa-brand-center/images-and-media/",
    source_url: "https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-topography-bathymetry",
    text: "Satellite imagery, with GEBCO bathymetry. NASA Earth Observatory.",
};

pub const BLACK_MARBLE: Credit = Credit {
    id: "nasa-black-marble",
    name: "NASA Earth at Night (Black Marble)",
    license: "US government work (NASA media guidelines)",
    license_url: "https://www.nasa.gov/nasa-brand-center/images-and-media/",
    source_url: "https://science.nasa.gov/earth/earth-observatory/earth-at-night/maps/",
    text: "Night lights, 2016. NASA Earth Observatory images by Joshua Stevens, using Suomi NPP VIIRS data from Miguel Román, NASA's Goddard Space Flight Center.",
};

pub const HYG: Credit = Credit {
    id: "hyg",
    name: "HYG star database",
    license: "CC BY-SA 4.0",
    license_url: "https://creativecommons.org/licenses/by-sa/4.0/",
    source_url: "https://codeberg.org/astronexus/hyg",
    text: "Stars of the sky view (positions, brightness, colors, names), v4.4 by David Nash (astronexus). Our converted star file is offered under the same license.",
};

pub const STELLARIUM: Credit = Credit {
    id: "stellarium-skycultures",
    name: "Stellarium sky cultures (western)",
    license: "CC BY-SA",
    license_url: "https://creativecommons.org/licenses/by-sa/4.0/",
    source_url: "https://github.com/Stellarium/stellarium-skycultures",
    text: "Constellation figures and names of the sky view. Our converted figure file (with HYG positions) is offered under CC BY-SA 4.0.",
};

/// Every dataset the app can show.
pub const DEEP_STAR_MAPS: Credit = Credit {
    id: "nasa-svs-deep-star-maps",
    name: "NASA SVS Deep Star Maps 2020",
    license: "US government work (NASA media guidelines)",
    license_url: "https://www.nasa.gov/nasa-brand-center/images-and-media/",
    source_url: "https://svs.gsfc.nasa.gov/4851",
    text: "The Milky Way of the sky view. NASA/Goddard Space Flight Center Scientific Visualization Studio. Gaia DR2: ESA/Gaia/DPAC.",
};

pub const DSS: Credit = Credit {
    id: "dss",
    name: "Digitized Sky Survey (DSS2, color)",
    license: "CC BY-NC-SA (non-commercial)",
    license_url: "https://archive.stsci.edu/dss/copyright.html",
    source_url: "https://archive.stsci.edu/dss/",
    text: "The deep-sky survey of the sky view. Digitized Sky Survey - STScI/NASA; colored and HiPS by CDS (CNRS/Unistra); tiles served by ESA ESASky and NASA/IPAC IRSA. Plates: POSS-II (Caltech), UK Schmidt (UK SERC/PPARC, Anglo-Australian Observatory).",
};

pub const JPL_HORIZONS: Credit = Credit {
    id: "jpl-horizons",
    name: "NASA/JPL Horizons and Small-Body Database",
    license: "US government service (no license published)",
    license_url: "https://ssd.jpl.nasa.gov/horizons/manual.html",
    source_url: "https://ssd-api.jpl.nasa.gov/doc/horizons.html",
    text: "Spacecraft positions and the orbits of dwarf planets, asteroids and comets in the solar system view, refreshed weekly. NASA/JPL Solar System Dynamics.",
};

pub const PLANET_MAPS: Credit = Credit {
    id: "nasa-planetary-maps",
    name: "NASA and USGS planetary maps",
    license: "US government works (NASA media guidelines, USGS public domain)",
    license_url: "https://www.nasa.gov/nasa-brand-center/images-and-media/",
    source_url: "https://svs.gsfc.nasa.gov/4720",
    text: "Surfaces of the Moon, Mars, Mercury and Jupiter in the solar system view. Moon: NASA/GSFC Scientific Visualization Studio, CGI Moon Kit (LRO). Mars: USGS Astrogeology and NASA, Viking MDIM 2.1. Mercury: NASA/JHUAPL/Carnegie Institution of Washington (MESSENGER). Jupiter: NASA/JPL/Space Science Institute (Cassini).",
};

pub const CLIOPATRIA: Credit = Credit {
    id: "cliopatria",
    name: "Cliopatria (Seshat Global History Databank)",
    license: "CC BY 4.0",
    license_url: "https://creativecommons.org/licenses/by/4.0/",
    source_url: "https://github.com/Seshat-Global-History-Databank/cliopatria",
    text: "Historical borders, 3400 BCE to 2024: polities from Cliopatria v0.2.1 (Bennett et al. 2025, Scientific Data), simplified and recolored. These maps are one version of the territory past polities held; borders, names and dates are often uncertain.",
};

pub const GLOTTOLOG: Credit = Credit {
    id: "glottolog",
    name: "Glottolog 5.4",
    license: "CC BY 4.0",
    license_url: "https://creativecommons.org/licenses/by/4.0/",
    source_url: "https://github.com/glottolog/glottolog-cldf",
    text: "Languages, their locations and families. Hammarström, Harald, Robert Forkel, Martin Haspelmath and Sebastian Bank, Glottolog 5.4, Leipzig: Max Planck Institute for Evolutionary Anthropology.",
};

pub const ALL: [&Credit; 11] = [
    &NATURAL_EARTH,
    &BLUE_MARBLE,
    &BLACK_MARBLE,
    &HYG,
    &STELLARIUM,
    &DEEP_STAR_MAPS,
    &DSS,
    &JPL_HORIZONS,
    &PLANET_MAPS,
    &CLIOPATRIA,
    &GLOTTOLOG,
];

/// The credits of the sky view; the survey's only while it is shown.
pub fn sky_credits(survey: bool) -> Vec<&'static Credit> {
    let mut v = vec![&HYG, &STELLARIUM, &DEEP_STAR_MAPS];
    if survey {
        v.push(&DSS);
    }
    v
}

/// The credits of the solar system view: JPL for the spacecraft and small
/// bodies, the stars and the Milky Way behind them (the planets are
/// computed from published elements, not a dataset).
pub fn solar_credits() -> Vec<&'static Credit> {
    vec![
        &JPL_HORIZONS,
        &PLANET_MAPS,
        &BLUE_MARBLE,
        &HYG,
        &DEEP_STAR_MAPS,
    ]
}

/// `base` plus the credits of the overlays that are on (the languages).
pub fn with_overlays(mut base: Vec<&'static Credit>, languages: bool) -> Vec<&'static Credit> {
    if languages {
        base.insert(0, &GLOTTOLOG);
    }
    base
}

/// The credits of the historical borders view: the modern base map and
/// Cliopatria.
pub fn history_credits() -> Vec<&'static Credit> {
    vec![&CLIOPATRIA, &NATURAL_EARTH]
}

/// The credits that apply while `layer` is shown. Natural Earth is behind
/// every view (map shape, labels, search, statistics, the Terrain layer's
/// relief); each NASA image only shows on its own layer.
pub fn visible_credits(layer: Layer) -> Vec<&'static Credit> {
    ALL.into_iter()
        .filter(|c| c.id != BLUE_MARBLE.id || layer == Layer::Imagery)
        .filter(|c| c.id != BLACK_MARBLE.id || layer == Layer::NightLights)
        .filter(|c| !sky_credits(true).iter().any(|s| s.id == c.id))
        // The solar view's own data (it shares the Blue Marble with the Earth).
        // Optional overlays add their own (see `with_overlays`).
        .filter(|c| ![JPL_HORIZONS.id, PLANET_MAPS.id, CLIOPATRIA.id, GLOTTOLOG.id].contains(&c.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_layer_credits_natural_earth() {
        for layer in Layer::ALL {
            let ids: Vec<_> = visible_credits(layer).iter().map(|c| c.id).collect();
            assert!(ids.contains(&"natural-earth"), "{layer:?}");
        }
    }

    #[test]
    fn imagery_adds_nasa_and_only_imagery() {
        for layer in Layer::ALL {
            let has = visible_credits(layer)
                .iter()
                .any(|c| c.id == BLUE_MARBLE.id);
            assert_eq!(has, layer == Layer::Imagery, "{layer:?}");
        }
    }

    #[test]
    fn night_lights_add_black_marble_and_only_there() {
        for layer in Layer::ALL {
            let ids: Vec<_> = visible_credits(layer).iter().map(|c| c.id).collect();
            assert_eq!(
                ids.contains(&BLACK_MARBLE.id),
                layer == Layer::NightLights,
                "{layer:?}"
            );
        }
    }

    #[test]
    fn the_sky_credits_its_stars_and_the_earth_does_not() {
        assert_eq!(
            sky_credits(false).iter().map(|c| c.id).collect::<Vec<_>>(),
            ["hyg", "stellarium-skycultures", "nasa-svs-deep-star-maps"]
        );
        assert_eq!(sky_credits(true).last().unwrap().id, "dss");
        assert_eq!(solar_credits()[0].id, "jpl-horizons");
        for layer in Layer::ALL {
            let ids: Vec<_> = visible_credits(layer).iter().map(|c| c.id).collect();
            assert!(!ids.contains(&JPL_HORIZONS.id));
        }
        for layer in Layer::ALL {
            let ids: Vec<_> = visible_credits(layer).iter().map(|c| c.id).collect();
            assert!(!ids.contains(&HYG.id) && !ids.contains(&STELLARIUM.id));
        }
    }

    #[test]
    fn no_duplicates_and_all_listed() {
        for layer in Layer::ALL {
            let v = visible_credits(layer);
            for (i, a) in v.iter().enumerate() {
                assert!(v[i + 1..].iter().all(|b| a.id != b.id), "{layer:?}");
                assert!(ALL.iter().any(|c| c.id == a.id));
            }
        }
        for (i, a) in ALL.iter().enumerate() {
            assert!(ALL[i + 1..].iter().all(|b| a.id != b.id));
        }
    }

    #[test]
    fn entries_are_complete_with_https_links() {
        for c in ALL {
            assert!(!c.name.is_empty() && !c.license.is_empty() && !c.text.is_empty());
            assert!(c.license_url.starts_with("https://"), "{}", c.id);
            assert!(c.source_url.starts_with("https://"), "{}", c.id);
        }
    }

    /// The "In use" table of DATA_LICENSES.md and `ALL` must agree.
    #[test]
    fn data_licenses_in_use_table_is_covered() {
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/DATA_LICENSES.md"))
            .expect("read DATA_LICENSES.md");
        let section = doc
            .split("## In use")
            .nth(1)
            .expect("In use section")
            .split("\n## ")
            .next()
            .unwrap();
        let mut rows = 0;
        for line in section.lines().filter(|l| l.starts_with("| [")) {
            rows += 1;
            let url = line
                .split("](")
                .nth(1)
                .and_then(|s| s.split(')').next())
                .expect("link in first column");
            assert!(
                ALL.iter().any(|c| c.source_url == url),
                "no credits entry for {url}"
            );
            // The `[key: id]` tag (shared with data-sources.lock.json) is the credit id.
            let key = line
                .split("[key: ")
                .nth(1)
                .and_then(|s| s.split(']').next())
                .expect("[key: id] tag in first column");
            assert!(
                ALL.iter().any(|c| c.id == key && c.source_url == url),
                "credits entry for {url} should have id {key}"
            );
        }
        assert_eq!(
            rows,
            ALL.len(),
            "credits and DATA_LICENSES.md differ in size"
        );
    }
}
