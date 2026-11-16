# Microsoft Store submission materials

This is a maintainer bundle for **Rusty's Snout - Firmware Explorer**. It contains
an **unsigned** Store package and listing materials. Use the normal EXE/MSI or
Linux release assets to install the direct-download edition.

## Submit to Partner Center

1. Extract this ZIP. Under `package/`, verify the MSIX's SHA-256 against its
   `.sha256` file; `.build-info.json` records its application/package versions,
   target and digest. CI checks that the digest agrees with both sidecars.
2. Open product **Rusty's Snout - Firmware Explorer** (`9NWM3QW158VZ`) in Partner
   Center. Upload the **contained `.msix`** under Packages. This ZIP is an
   organizational bundle, not the package to upload. Microsoft signs the MSIX
   during Store publication.
3. Use `docs/store-listing.md` for listing text, support/privacy URLs and reviewer
   steps. Complete properties, availability/pricing, age ratings and certification
   notes, explaining normal desktop execution with `runFullTrust`.
4. Upload the listing PNGs from `crates/snout/packaging/windows/store-listing/`.
   The original icon files and manifest template are included for reference.
   Manifest package assets are already contained inside the MSIX.
5. Capture current screenshots of the installed Windows Store edition before
   submission. The separate `snout-screenshot.png` release asset is a Linux
   fixture-based reference capture. This archive does not require or include
   that capture unless a screenshot was explicitly supplied to the archiver.
6. Verify the public privacy-policy URL, run Windows installation/upgrade and
   certification checks described in `docs/technical-reference.md`, then review
   and submit the listing. CI does not submit to Partner Center automatically.

The package manifest identity is `rustypig91.RustysSnout-FirmwareExplorer`, with
publisher `CN=B41BE868-9458-4B44-AE02-EA569AC9D53B`. The package version comes from
the app version with `.0` appended; no separate Store version is configured.

Each successful Store workflow builds this bundle. Tagged GitHub releases attach
the same completed `store-submission.zip` after all builds, archival and privacy
deployment succeed. A missing package, checksum, metadata or artwork fails the
archive step and prevents release publication.
