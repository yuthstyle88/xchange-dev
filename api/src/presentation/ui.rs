//! UiAssets compiled with rust-embed.

#[derive(rust_embed::RustEmbed)]
#[folder = "../web/dist/"]
pub struct UiAssets;

impl UiAssets {
    pub fn built() -> bool {
        Self::get(".vite/manifest.json").is_some()
    }
}
