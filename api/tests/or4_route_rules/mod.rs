//! Shared OR-4b route-file rule (`route_file_verdict`) used by `or4_routes_guard_test` and
//! `or4_route_file_rule_test` (#2008).

const TRAITS: &[&str] = &[
    "BranchView",
    "PagedView",
    "ComputeAction",
    "CommandAction",
    "OrgView",
];

const RAW_ROUTE_FORMS: &[&str] = &[
    ".route(",
    "web::resource(",
    "web::get()",
    "web::post()",
    "web::put()",
    "web::delete()",
    "web::patch()",
    "#[get(",
    "#[post(",
    "#[put(",
    "#[delete(",
    "#[patch(",
    ".service(",
];

/// Verifies that a v3 route file (`code` with `//` comment lines removed) obeys OR-4b ruling (b):
/// - (i) `const INHERITS:` on an impl of `BranchView`/`PagedView`/`ComputeAction`/`CommandAction`/`OrgView`, or
/// - (ii) `const PUBLIC_ROUTE:`, or
/// - (iii) a pure `CommandEndpoint` registrar (at least one `CommandEndpoint::register(` call and no raw route forms).
pub fn route_file_verdict(code: &str) -> Result<(), String> {
    let inherits = code.contains("const INHERITS:");
    let on_a_trait = TRAITS.iter().any(|t| code.contains(&format!("{t} for ")));
    if inherits && !on_a_trait {
        return Err(
            "INHERITS declared outside an impl of BranchView/PagedView/ComputeAction/CommandAction"
                .to_string(),
        );
    }
    if inherits || code.contains("const PUBLIC_ROUTE:") {
        return Ok(());
    }
    if code.contains("CommandEndpoint::register(") {
        if let Some(raw) = RAW_ROUTE_FORMS.iter().find(|form| code.contains(**form)) {
            return Err(format!(
                "CommandEndpoint registrar contains raw route form `{raw}`"
            ));
        }
        return Ok(());
    }
    Err("a v3 route file declares neither INHERITS nor PUBLIC_ROUTE".to_string())
}
