use std::path::Path;

use xchange::presentation::screens::ScreenTable;

#[test]
fn the_compiled_screen_table_is_the_one_web_dist_declares() {
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../web/dist");
    let manifest = dist.join(".vite/manifest.json");
    let require_web_dist = std::env::var("XC_REQUIRE_WEB_DIST").as_deref() == Ok("1");

    if !manifest.is_file() {
        if require_web_dist {
            panic!(
                "web/dist/.vite/manifest.json is missing: build web first (run `npm --prefix web ci && npm --prefix web run check`)"
            );
        }
        // When XC_REQUIRE_WEB_DIST is unset (e.g. a local cargo test run without web built),
        // there is nothing to compare against.
        return;
    }

    let dist_table =
        ScreenTable::from_dist(&dist).expect("failed to load screen table from web/dist");
    let compiled = ScreenTable::compiled();

    let compiled_entries = compiled.entries();
    let dist_entries = dist_table.entries();

    if compiled_entries != dist_entries {
        let mut differing_paths = Vec::new();
        for d in dist_entries {
            if !compiled_entries.contains(d) {
                differing_paths.push(d.path.as_str());
            }
        }
        for c in compiled_entries {
            if !dist_entries.contains(c) && !differing_paths.contains(&c.path.as_str()) {
                differing_paths.push(c.path.as_str());
            }
        }
        panic!(
            "the compiled screen table does not match web/dist (differing paths: {differing_paths:?})\ncompiled: {compiled_entries:?}\ndist: {dist_entries:?}"
        );
    }
}
