# License Audit

Date: 2026-07-28

This is an engineering inventory, not legal advice.

## Confirmed metadata

- Root and publishable framework manifests declare Apache-2.0.
- `LICENSE-APACHE` retains Longbridge copyright and attribution.
- GPUI is a separate BumpyClock fork with its own provenance and publication
  work; its notices must remain with its source and package artifacts.

## Assets requiring owner review

- `crates/assets/assets/icons/` contains SVGs identified as Lucide assets.
- The repository has no tracked Lucide license text, version record, source
  acquisition record, or package notice for those SVGs.
- `crates/assets/README.md` still links to the historical Longbridge repository.

Before publishing a framework package that includes these assets, an owner must
confirm each asset's source, version, redistribution terms, and required notice.
Add only the resulting verified notice and attribution; do not change the
project license or infer asset permissions from SVG class names.

## Publication gate

Crates.io publication remains blocked until the asset review above and the GPUI
engine package identity/ownership checks in `compatibility.toml` are resolved.
