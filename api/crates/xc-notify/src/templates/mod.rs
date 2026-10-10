//! One file per template (design `customer-app.md` §5). `xc-notify/build.rs` generates the list
//! from the files of this folder; `mod.rs` is not a template.

include!(concat!(env!("OUT_DIR"), "/templates.rs"));
