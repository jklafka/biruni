//! Data files by name, through the manifest.
//!
//! The build (`scripts/hash_data.py`, a Trunk post-build hook) moves every
//! file from `assets/data/` to `data/files/<stem>.<hash>.<ext>` and writes
//! `data/manifest.json`, which maps each original name to its hashed path:
//!
//! ```json
//! { "version": 1, "files": { "labels.tsv": "files/labels.0123456789abcdef.tsv" } }
//! ```
//!
//! Hashed files never change, so they are cached as immutable; only the small
//! manifest is revalidated. A data update then switches every file at once,
//! and the old files can stay for pages that are still open. Paths in the
//! manifest are relative to the manifest itself, or absolute URLs, so the
//! data can later move to another host (R2) without changing the app.
//!
//! The manifest is fetched once (it is preloaded from `index.html`) and
//! shared by every caller. `Manifest` holds the browser-free part: checking
//! the entries and resolving a name to a URL.

use std::cell::RefCell;
use wasm_bindgen::{JsCast, JsValue};

/// Where the app looks for the manifest, relative to the page.
pub const MANIFEST_URL: &str = "data/manifest.json";

/// The manifest format this build understands.
pub const MANIFEST_VERSION: f64 = 1.0;

#[derive(Debug)]
pub struct Manifest {
    /// Directory of the manifest URL, with a trailing `/` (or empty).
    base: String,
    files: Vec<(String, String)>,
}

impl Manifest {
    /// Check the entries of a manifest loaded from `manifest_url`.
    pub fn new(manifest_url: &str, files: Vec<(String, String)>) -> Result<Manifest, String> {
        for (i, (name, path)) in files.iter().enumerate() {
            if name.is_empty() || name.contains('/') || name.contains('\\') {
                return Err(format!("manifest: bad file name {name:?}"));
            }
            if files[..i].iter().any(|(n, _)| n == name) {
                return Err(format!("manifest: {name} is listed twice"));
            }
            let bad_path = path.is_empty()
                || path.contains('\\')
                || path.chars().any(char::is_whitespace)
                || path.split('/').any(|seg| seg == "..");
            if bad_path {
                return Err(format!("manifest: bad path {path:?} for {name}"));
            }
        }
        Ok(Manifest {
            base: base_of(manifest_url).to_string(),
            files,
        })
    }

    /// URL of the data file `name` (its name in `assets/data/`).
    pub fn url(&self, name: &str) -> Result<String, String> {
        let (_, path) = self
            .files
            .iter()
            .find(|(n, _)| n == name)
            .ok_or_else(|| format!("manifest has no entry for {name}"))?;
        Ok(resolve(&self.base, path))
    }
}

/// Everything up to and including the last `/` of the URL's path; empty
/// when there is none. A query or fragment is ignored.
fn base_of(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let path = &url[..end];
    path.rfind('/').map_or("", |i| &path[..=i])
}

/// `path` relative to `base`, unless it is already absolute (has a scheme,
/// or starts with `/`, which includes protocol-relative `//host/`).
fn resolve(base: &str, path: &str) -> String {
    let has_scheme = path
        .split_once(':')
        .is_some_and(|(scheme, _)| is_scheme(scheme));
    if has_scheme || path.starts_with('/') {
        path.to_string()
    } else {
        format!("{base}{path}")
    }
}

/// RFC 3986: a letter, then letters, digits, `+`, `-` or `.`.
fn is_scheme(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

thread_local! {
    /// The manifest text, fetched once and shared by concurrent callers.
    static MANIFEST: RefCell<Option<js_sys::Promise>> = const { RefCell::new(None) };
}

fn manifest_text() -> js_sys::Promise {
    MANIFEST.with(|m| {
        m.borrow_mut()
            .get_or_insert_with(|| {
                wasm_bindgen_futures::future_to_promise(async {
                    let bytes = crate::geo::fetch_bytes(MANIFEST_URL).await?;
                    let text = String::from_utf8(bytes)
                        .map_err(|_| JsValue::from_str("data/manifest.json is not UTF-8"))?;
                    Ok(JsValue::from_str(&text))
                })
            })
            .clone()
    })
}

async fn manifest() -> Result<Manifest, JsValue> {
    let loaded = wasm_bindgen_futures::JsFuture::from(manifest_text()).await;
    let text = match loaded {
        Ok(t) => t,
        Err(e) => {
            // Let a later call try again.
            MANIFEST.with(|m| m.borrow_mut().take());
            return Err(e);
        }
    };
    let text = text.as_string().ok_or("manifest text")?;
    let json = js_sys::JSON::parse(&text)
        .map_err(|_| JsValue::from_str("data/manifest.json is not valid JSON"))?;
    let version = js_sys::Reflect::get(&json, &"version".into())?.as_f64();
    if version != Some(MANIFEST_VERSION) {
        return Err(format!("data/manifest.json: unsupported version {version:?}").into());
    }
    let files = js_sys::Reflect::get(&json, &"files".into())?;
    let files: js_sys::Object = files
        .dyn_into()
        .map_err(|_| JsValue::from_str("data/manifest.json: no files"))?;
    let mut pairs = Vec::new();
    for entry in js_sys::Object::entries(&files).iter() {
        let entry: js_sys::Array = entry.dyn_into()?;
        let name = entry.get(0).as_string().ok_or("manifest name")?;
        let path = entry
            .get(1)
            .as_string()
            .ok_or_else(|| JsValue::from_str(&format!("manifest: path of {name}")))?;
        pairs.push((name, path));
    }
    Manifest::new(MANIFEST_URL, pairs).map_err(|e| JsValue::from_str(&e))
}

/// URL of the data file `name`, from the manifest.
pub async fn url(name: &str) -> Result<String, JsValue> {
    manifest()
        .await?
        .url(name)
        .map_err(|e| JsValue::from_str(&e))
}

/// Fetch the data file `name` (its name in `assets/data/`).
pub async fn fetch(name: &str) -> Result<Vec<u8>, JsValue> {
    crate::geo::fetch_bytes(&url(name).await?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn resolves_relative_paths_against_the_manifest_directory() {
        let m = Manifest::new(
            MANIFEST_URL,
            pairs(&[("labels.tsv", "files/labels.0123456789abcdef.tsv")]),
        )
        .unwrap();
        assert_eq!(
            m.url("labels.tsv").unwrap(),
            "data/files/labels.0123456789abcdef.tsv"
        );
    }

    #[test]
    fn keeps_absolute_urls() {
        let m = Manifest::new(
            "https://data.example.com/earth/manifest.json?v=2#x",
            pairs(&[
                ("a.bin", "https://cdn.example.com/a.1.bin"),
                ("b.bin", "//cdn.example.com/b.1.bin"),
                ("c.bin", "/root/c.1.bin"),
                ("d.bin", "files/d.1.bin"),
            ]),
        )
        .unwrap();
        assert_eq!(m.url("a.bin").unwrap(), "https://cdn.example.com/a.1.bin");
        assert_eq!(m.url("b.bin").unwrap(), "//cdn.example.com/b.1.bin");
        assert_eq!(m.url("c.bin").unwrap(), "/root/c.1.bin");
        assert_eq!(
            m.url("d.bin").unwrap(),
            "https://data.example.com/earth/files/d.1.bin"
        );
    }

    #[test]
    fn a_colon_later_in_a_relative_path_is_not_a_scheme() {
        assert_eq!(resolve("data/", "files/a:b.bin"), "data/files/a:b.bin");
        assert_eq!(resolve("data/", "1x:a.bin"), "data/1x:a.bin");
    }

    #[test]
    fn manifest_without_a_directory_resolves_against_the_page() {
        let m = Manifest::new("manifest.json", pairs(&[("a.bin", "a.1.bin")])).unwrap();
        assert_eq!(m.url("a.bin").unwrap(), "a.1.bin");
    }

    #[test]
    fn unknown_names_are_errors() {
        let m = Manifest::new(MANIFEST_URL, pairs(&[("a.bin", "a.1.bin")])).unwrap();
        assert!(m.url("b.bin").unwrap_err().contains("b.bin"));
        assert!(m.url("").is_err());
    }

    #[test]
    fn rejects_malformed_entries() {
        for bad in [
            ("", "files/a.bin"),
            ("dir/a.bin", "files/a.bin"),
            ("a\\b.bin", "files/a.bin"),
            ("a.bin", ""),
            ("a.bin", "../secret.bin"),
            ("a.bin", "files/../../a.bin"),
            ("a.bin", "files\\a.bin"),
            ("a.bin", "files/a b.bin"),
        ] {
            assert!(
                Manifest::new(MANIFEST_URL, pairs(&[bad])).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(Manifest::new(
            MANIFEST_URL,
            pairs(&[("a.bin", "files/a.1.bin"), ("a.bin", "files/a.2.bin")])
        )
        .is_err());
    }

    #[test]
    fn empty_manifest_has_no_files() {
        let m = Manifest::new(MANIFEST_URL, Vec::new()).unwrap();
        assert!(m.url("a.bin").is_err());
    }
}
