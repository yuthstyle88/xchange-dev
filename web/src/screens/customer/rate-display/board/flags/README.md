# Vendored Flag Icons

4x3 country/region flag SVGs vendored from [`lipis/flag-icons`](https://github.com/lipis/flag-icons):

- **Release tag:** `v7.6.0` (`0d8909b00aa8418802a5abf2afa82356b129a7ef`)
- **Source path:** `flags/4x3/<cc>.svg`
- **Licence:** MIT — see [`./LICENSE`](./LICENSE)

Each non-`X*` ISO 4217 currency seeded in `api/migrations/20261007100000_branches_rates.sql` maps via `currency.slice(0, 2).toLowerCase()` to `<cc>.svg` in this directory (including `EUR` → `eu.svg`). Supranational / precious-metal `X*` codes (`XAF`, `XAG`, `XAU`, `XCD`, `XCG`, `XOF`, `XPF`) have no country flag and render the `.xc-rb-code-box` fallback.
