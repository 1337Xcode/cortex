//! The map's page, scripts and styles, embedded in the binary when it was built: the daemon's
//! build script writes the table from the UI's build folder, so the map needs no network and no
//! files beside the binary. A binary built before the UI was has an empty table, and the map's
//! address then serves [`WITHOUT_MAP`], a page that says so.

/// One embedded file.
#[derive(Debug)]
pub(crate) struct Asset {
    /// Its path under the map's address, with `/` separators: `assets/index-1a2b.js`.
    pub(crate) path: &'static str,
    /// The content type it is served with.
    pub(crate) content_type: &'static str,
    /// Its bytes.
    pub(crate) bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/map_assets.rs"));

/// The file the map's page is.
const PAGE: &str = "index.html";

/// The page served at the map's address when this binary holds none of the map's files.
pub(crate) const WITHOUT_MAP: &str = "<!doctype html>\n<html lang=\"en\">\n<head><meta \
    charset=\"utf-8\"><title>Cortex map</title></head>\n<body><p>This build of Cortex was made \
    without the map's files, so there is no map to show. A build made after the map's interface \
    was built includes it.</p></body>\n</html>\n";

/// The file of `table` served at `path`, the part of a request's path after the token: the map's
/// page for the empty path; none for a path the table does not hold.
pub(crate) fn find(table: &'static [Asset], path: &str) -> Option<&'static Asset> {
    let path = if path.is_empty() { PAGE } else { path };
    table
        .binary_search_by(|asset| asset.path.cmp(path))
        .ok()
        .and_then(|found| table.get(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of two files, sorted by path as the build script writes it.
    static TWO: [Asset; 2] = [
        Asset {
            path: "assets/map.js",
            content_type: "text/javascript; charset=utf-8",
            bytes: b"run()",
        },
        Asset {
            path: "index.html",
            content_type: "text/html; charset=utf-8",
            bytes: b"<p>map</p>",
        },
    ];

    /// The empty path finds the page, each path its own file, and nothing else matches: not a
    /// path differing in case, not a folder, not a path climbing out.
    #[test]
    fn each_path_finds_its_file_and_no_other() {
        let path_of = |path: &str| find(&TWO, path).map(|asset| asset.path);
        assert_eq!(path_of(""), Some("index.html"));
        assert_eq!(path_of("index.html"), Some("index.html"));
        assert_eq!(path_of("assets/map.js"), Some("assets/map.js"));
        for missing in ["INDEX.HTML", "assets", "assets/", "../index.html", "x.js"] {
            assert_eq!(path_of(missing), None, "{missing}");
        }
        assert!(find(&[], "").is_none());
    }
}
