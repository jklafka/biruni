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
    text: "Coastlines, borders, countries, places, labels and country statistics. Made with Natural Earth.",
};

pub const BLUE_MARBLE: Credit = Credit {
    id: "nasa-blue-marble",
    name: "NASA Blue Marble: Next Generation",
    license: "US government work (NASA media guidelines)",
    license_url: "https://www.nasa.gov/nasa-brand-center/images-and-media/",
    source_url: "https://science.nasa.gov/earth/earth-observatory/blue-marble-next-generation/base-topography-bathymetry",
    text: "Satellite imagery, with GEBCO bathymetry. NASA Earth Observatory.",
};

/// Every dataset the app can show.
pub const ALL: [&Credit; 2] = [&NATURAL_EARTH, &BLUE_MARBLE];

/// The credits that apply while `layer` is shown. Natural Earth is behind
/// every view (map shape, labels, search, statistics); the satellite imagery
/// only shows on the Satellite layer.
pub fn visible_credits(layer: Layer) -> Vec<&'static Credit> {
    ALL.into_iter()
        .filter(|c| c.id != BLUE_MARBLE.id || layer == Layer::Imagery)
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
        }
        assert_eq!(
            rows,
            ALL.len(),
            "credits and DATA_LICENSES.md differ in size"
        );
    }
}
