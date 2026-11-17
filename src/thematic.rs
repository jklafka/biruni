//! Thematic (choropleth) layers: color each country by a statistic.
//!
//! Browser-free. Three themes use attributes of Natural Earth's admin-0
//! countries (public domain): population (`POP_EST`), GDP per capita
//! (`GDP_MD` / `POP_EST`) and income group (`INCOME_GRP`). Values are binned
//! into fixed classes, so the legend can name exact ranges. A country's class
//! is a small integer that indexes the theme's palette in `globe.frag`
//! (`u_theme`); [`NO_DATA`] marks a missing or unusable value.
//!
//! The palette is sampled from viridis: sequential, perceptually even, safe for
//! the common color-vision deficiencies, and every step differs in lightness, so
//! it still reads on the dark ocean. "No data" is a neutral grey.

/// Class index of a country with no usable value. Classes are `0..NO_DATA`,
/// and `u_theme` in `globe.frag` has `NO_DATA + 1` entries.
pub const NO_DATA: u8 = 7;

/// Color of [`NO_DATA`], sRGB. Shared by the shader table and the legend.
pub const NO_DATA_COLOR: [u8; 3] = [0x7a, 0x7f, 0x87];

/// Seven viridis steps, dark to light (low to high).
const RAMP7: [[u8; 3]; 7] = [
    [0x46, 0x32, 0x7e],
    [0x36, 0x5c, 0x8d],
    [0x27, 0x7f, 0x8e],
    [0x1f, 0xa1, 0x87],
    [0x4a, 0xc1, 0x6d],
    [0xa0, 0xda, 0x39],
    [0xfd, 0xe7, 0x25],
];
/// Five steps for the ordered income groups.
const RAMP5: [[u8; 3]; 5] = [
    [0x46, 0x32, 0x7e],
    [0x2f, 0x6b, 0x8e],
    [0x1f, 0xa1, 0x87],
    [0x85, 0xd5, 0x4a],
    [0xfd, 0xe7, 0x25],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Theme {
    Population,
    GdpPerCapita,
    Income,
}

/// The attributes a theme reads from a country.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub population: u64,
    /// Gross domestic product, million US dollars; 0 when unknown.
    pub gdp_md: u64,
    /// 1 (low income) to 5 (high income, OECD); 0 when unknown.
    pub income: u8,
    pub pop_year: u16,
    pub gdp_year: u16,
}

const POP_BREAKS: [f64; 6] = [1e6, 1e7, 3e7, 1e8, 3e8, 1e9];
const GDP_BREAKS: [f64; 6] = [1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0, 40_000.0];
const INCOME_LABELS: [&str; 5] = [
    "Low income",
    "Lower middle income",
    "Upper middle income",
    "High income, non-OECD",
    "High income, OECD",
];

impl Theme {
    #[cfg(test)]
    pub const ALL: [Theme; 3] = [Theme::Population, Theme::GdpPerCapita, Theme::Income];

    /// Title of the legend.
    pub fn title(self) -> &'static str {
        match self {
            Theme::Population => "Population",
            Theme::GdpPerCapita => "GDP per capita (US$)",
            Theme::Income => "Income group",
        }
    }

    fn breaks(self) -> &'static [f64] {
        match self {
            Theme::Population => &POP_BREAKS,
            Theme::GdpPerCapita => &GDP_BREAKS,
            Theme::Income => &[],
        }
    }

    /// Number of classes, not counting "no data".
    #[cfg(test)]
    pub fn class_count(self) -> usize {
        match self {
            Theme::Income => INCOME_LABELS.len(),
            _ => self.breaks().len() + 1,
        }
    }

    /// The theme's numeric value for a country, if it has a usable one.
    /// Zero or negative populations and GDP, and a missing population for
    /// GDP per capita, count as missing (a log scale has no place for them).
    pub fn value(self, s: &Stats) -> Option<f64> {
        let v = match self {
            Theme::Population => s.population as f64,
            Theme::GdpPerCapita => {
                if s.gdp_md == 0 || s.population == 0 {
                    return None;
                }
                s.gdp_md as f64 * 1e6 / s.population as f64
            }
            Theme::Income => f64::from(s.income),
        };
        (v.is_finite() && v > 0.0).then_some(v)
    }

    /// Class of a numeric value: the number of breaks it reaches. NaN,
    /// infinities and values at or below zero are [`NO_DATA`].
    pub fn classify(self, value: f64) -> u8 {
        if !value.is_finite() || value <= 0.0 {
            return NO_DATA;
        }
        match self {
            Theme::Income => {
                let rank = value.round();
                if (1.0..=5.0).contains(&rank) {
                    rank as u8 - 1
                } else {
                    NO_DATA
                }
            }
            _ => self.breaks().iter().filter(|b| value >= **b).count() as u8,
        }
    }

    pub fn class_of(self, s: &Stats) -> u8 {
        self.value(s).map_or(NO_DATA, |v| self.classify(v))
    }

    /// Palette for the shader: `NO_DATA + 1` colors, indexed by class.
    pub fn palette(self) -> [[u8; 3]; NO_DATA as usize + 1] {
        let mut out = [NO_DATA_COLOR; NO_DATA as usize + 1];
        match self {
            Theme::Income => out[..5].copy_from_slice(&RAMP5),
            _ => out[..7].copy_from_slice(&RAMP7),
        }
        out
    }

    /// Legend rows, low to high: the class and its text. "No data" is not
    /// included (the legend adds it).
    pub fn legend(self) -> Vec<(u8, String)> {
        match self {
            Theme::Income => INCOME_LABELS
                .iter()
                .enumerate()
                .map(|(i, l)| (i as u8, (*l).to_owned()))
                .collect(),
            _ => {
                let b = self.breaks();
                let fmt = |v: f64| match self {
                    Theme::Population => format_compact(v, ""),
                    _ => format_compact(v, "$"),
                };
                (0..=b.len())
                    .map(|i| {
                        let text = if i == 0 {
                            format!("< {}", fmt(b[0]))
                        } else if i == b.len() {
                            format!("\u{2265} {}", fmt(b[i - 1]))
                        } else {
                            format!("{} \u{2013} {}", fmt(b[i - 1]), fmt(b[i]))
                        };
                        (i as u8, text)
                    })
                    .collect()
            }
        }
    }

    /// Country value for the info panel, with its year where there is one.
    pub fn describe(self, s: &Stats) -> String {
        match self {
            Theme::Population => {
                with_year(crate::country::format_population(s.population), s.pop_year)
            }
            Theme::GdpPerCapita => match self.value(s) {
                Some(v) => with_year(
                    format!("${}", crate::country::format_population(v.round() as u64)),
                    s.gdp_year,
                ),
                None => "No data".to_owned(),
            },
            Theme::Income => match s.income {
                1..=5 => INCOME_LABELS[usize::from(s.income) - 1].to_owned(),
                _ => "No data".to_owned(),
            },
        }
    }
}

fn with_year(text: String, year: u16) -> String {
    if year == 0 {
        text
    } else {
        format!("{text} ({year})")
    }
}

/// `1_000_000` becomes `"1M"`, `2_500` with prefix `"$"` becomes `"$2.5k"`.
pub fn format_compact(v: f64, prefix: &str) -> String {
    let (n, unit) = if v >= 1e9 {
        (v / 1e9, "B")
    } else if v >= 1e6 {
        (v / 1e6, "M")
    } else if v >= 1e3 {
        (v / 1e3, "k")
    } else {
        (v, "")
    };
    if n.fract() == 0.0 {
        format!("{prefix}{n:.0}{unit}")
    } else {
        format!("{prefix}{n:.1}{unit}")
    }
}

/// The most common nonzero year, the earlier one on a tie; 0 when there is none.
pub fn mode_year(years: impl IntoIterator<Item = u16>) -> u16 {
    let mut counts = std::collections::BTreeMap::new();
    for y in years.into_iter().filter(|y| *y != 0) {
        *counts.entry(y).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|&(y, n)| (n, std::cmp::Reverse(y)))
        .map_or(0, |(y, _)| y)
}

/// Source line under the legend, naming the year most countries share.
pub fn source_note(year: u16) -> String {
    if year == 0 {
        "Natural Earth estimates".to_owned()
    } else {
        format!("Natural Earth estimates, mostly {year}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(pop: u64, gdp: u64, income: u8) -> Stats {
        Stats {
            population: pop,
            gdp_md: gdp,
            income,
            pop_year: 2019,
            gdp_year: 2019,
        }
    }

    #[test]
    fn population_classes_follow_the_breaks() {
        let t = Theme::Population;
        assert_eq!(t.classify(1.0), 0);
        assert_eq!(t.classify(999_999.0), 0);
        assert_eq!(t.classify(1e6), 1, "a break belongs to the class above it");
        assert_eq!(t.classify(9_999_999.0), 1);
        assert_eq!(t.classify(1.4e9), 6);
        assert_eq!(t.class_count(), 7);
    }

    #[test]
    fn unusable_values_are_no_data() {
        for t in Theme::ALL {
            assert_eq!(t.classify(f64::NAN), NO_DATA);
            assert_eq!(t.classify(f64::INFINITY), NO_DATA);
            assert_eq!(t.classify(f64::NEG_INFINITY), NO_DATA);
            assert_eq!(t.classify(0.0), NO_DATA);
            assert_eq!(t.classify(-5.0), NO_DATA);
            assert_eq!(t.class_of(&Stats::default()), NO_DATA);
        }
        assert_eq!(Theme::Income.classify(6.0), NO_DATA);
    }

    #[test]
    fn gdp_per_capita_needs_both_inputs() {
        let t = Theme::GdpPerCapita;
        // 21,440 million over 14.6 million people is about $1,464.
        let zim = stats(14_645_468, 21_440, 1);
        let v = t.value(&zim).unwrap();
        assert!((v - 1_463.9).abs() < 1.0, "{v}");
        assert_eq!(t.class_of(&zim), 1);
        assert_eq!(t.class_of(&stats(0, 500, 3)), NO_DATA, "no population");
        assert_eq!(t.class_of(&stats(10, 0, 3)), NO_DATA, "no GDP");
        // $100,000 per person is in the top class.
        assert_eq!(t.class_of(&stats(1_000_000, 100_000, 5)), 6);
    }

    #[test]
    fn income_ranks_map_to_classes() {
        let t = Theme::Income;
        for rank in 1..=5u8 {
            assert_eq!(t.class_of(&stats(1, 1, rank)), rank - 1);
        }
        assert_eq!(t.class_of(&stats(1, 1, 0)), NO_DATA);
        assert_eq!(t.class_of(&stats(1, 1, 9)), NO_DATA);
        assert_eq!(t.class_count(), 5);
    }

    #[test]
    fn legend_matches_classes_and_palette() {
        for t in Theme::ALL {
            let legend = t.legend();
            assert_eq!(legend.len(), t.class_count());
            let pal = t.palette();
            assert_eq!(pal[usize::from(NO_DATA)], NO_DATA_COLOR);
            for (i, (class, text)) in legend.iter().enumerate() {
                assert_eq!(usize::from(*class), i);
                assert!(!text.is_empty());
                // Classes get distinct colors, none equal to "no data".
                assert_ne!(pal[i], NO_DATA_COLOR);
                for earlier in &pal[..i] {
                    assert_ne!(&pal[i], earlier);
                }
            }
        }
        let pop = Theme::Population.legend();
        assert_eq!(pop[0].1, "< 1M");
        assert_eq!(pop[1].1, "1M \u{2013} 10M");
        assert_eq!(pop[6].1, "\u{2265} 1B");
        let gdp = Theme::GdpPerCapita.legend();
        assert_eq!(gdp[0].1, "< $1k");
        assert_eq!(gdp[6].1, "\u{2265} $40k");
    }

    #[test]
    fn crossing_each_break_moves_up_one_class() {
        for t in [Theme::Population, Theme::GdpPerCapita] {
            let mut last = 0;
            for b in t.breaks() {
                assert_eq!(t.classify(b - 0.5), last);
                last += 1;
                assert_eq!(t.classify(*b), last);
            }
        }
    }

    #[test]
    fn formatting() {
        assert_eq!(format_compact(1e6, ""), "1M");
        assert_eq!(format_compact(2_500.0, "$"), "$2.5k");
        assert_eq!(format_compact(1e9, ""), "1B");
        assert_eq!(format_compact(30e6, ""), "30M");
        assert_eq!(format_compact(500.0, "$"), "$500");
        let s = stats(14_645_468, 21_440, 1);
        assert_eq!(Theme::Population.describe(&s), "14,645,468 (2019)");
        assert_eq!(Theme::GdpPerCapita.describe(&s), "$1,464 (2019)");
        assert_eq!(Theme::Income.describe(&s), "Low income");
        assert_eq!(Theme::GdpPerCapita.describe(&Stats::default()), "No data");
        assert_eq!(Theme::Income.describe(&Stats::default()), "No data");
        let no_year = Stats {
            population: 5,
            ..Stats::default()
        };
        assert_eq!(Theme::Population.describe(&no_year), "5");
    }

    #[test]
    fn mode_year_picks_the_most_common() {
        assert_eq!(mode_year([2019, 2018, 2019, 0, 0, 0]), 2019);
        assert_eq!(
            mode_year([2018, 2019]),
            2018,
            "tie goes to the earlier year"
        );
        assert_eq!(mode_year([0, 0]), 0);
        assert_eq!(mode_year([]), 0);
        assert_eq!(source_note(2019), "Natural Earth estimates, mostly 2019");
        assert_eq!(source_note(0), "Natural Earth estimates");
    }
}
