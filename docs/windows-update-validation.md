# Windows update validation (issue #6)

The Linux tests exercise ownership matching, installer link selection, update
policy, and the MSI replacement guard. They do not validate native MSI
registration, elevation, or uninstall behavior. Run the following on a Windows
test machine with two distinct release versions and administrator access.
Use a newer published release for the update dialog checks.

Run `cargo test -p firmware-gui --locked` first. This includes the native
Windows Installer ownership test, both Setup helper scope tests, and isolated
portable replacement. Do not use `Win32_Product` queries, which can trigger MSI
repairs.

## Per-machine MSI

1. Install the older MSI, including a run using a custom destination. Launch
   its installed GUI and check for updates. Expect the Windows Installer
   explanation and **Download MSI installer**, with no **Update** button.
   Confirm the link targets the newer release's
   `snout-v<VERSION>-x86_64-pc-windows-msvc.msi` asset.
2. Probe the actual installed executable against native MSI registration:

   ```powershell
   $env:SNOUT_MSI_EXECUTABLE = 'C:\Program Files\Rusty''s Snout - Firmware Explorer\firmware-gui.exe'
   $env:SNOUT_EXPECT_MSI = '1'
   cargo test -p firmware-gui --locked update::msi::tests::native_ownership_matches_the_integration_environment -- --exact
   ```

   Substitute the custom destination when applicable.
   Also probe a hard link to the installed executable on the same volume (create
   it with `New-Item -ItemType HardLink -Path <alias> -Target <installed-exe>`).
   Expect ownership `true` for the alias; an independent copy of the same file
   must return `false`. Remove the alias after the probe. This checks file
   identity rather than canonical path spelling.
   Also temporarily move `License.rtf` out of the MSI installation folder and
   repeat the probe and update-dialog check. Expect ownership `true` and
   **Download MSI installer** even with the ancillary component missing; restore
   the license afterward. A separate portable copy must still block replacement
   if that missing registered path leaves its ownership uncertain.
3. Close Snout, download and run the newer MSI, approving elevation. Verify
   **About** reports the new version and Windows Installed Apps shows exactly
   one Snout MSI entry with the new version. Run the ownership probe again.
   Verify the Start Menu shortcut opens the upgraded app.
4. Uninstall through Windows Installed Apps. Verify the executable, advertised
   shortcut, and MSI uninstall entry are removed. Reinstall for coexistence tests.

## Setup and portable copies

1. Install Setup in **current user** scope in a separate folder while MSI is
   installed. Set `SNOUT_MSI_EXECUTABLE` to that Setup executable and
   `SNOUT_EXPECT_MSI` to `0`; rerun the exact native test above. Launch Setup's
   GUI, check for updates, and expect **Update**. Upgrade and verify **About**,
   Installed Apps version, original destination/scope, and uninstall behavior.
2. Repeat for Setup's **all users** scope, approving elevation. Keep its folder
   separate from MSI and current-user Setup. Verify one uninstall entry for that
   scope, with the upgraded version, and that uninstall removes that copy only.
3. Copy the portable executable to a separate folder. Run the native probe with
   that path and expected `0` while MSI remains installed. Launch it, update,
   and verify **About**. Confirm the MSI and Setup registrations and their
   binaries remain unchanged. Also test a portable folder under Program Files:
   it must remain classified as portable, although write permissions may prevent
   replacement.
4. Clear probe settings after validation:

   ```powershell
   Remove-Item Env:SNOUT_MSI_EXECUTABLE, Env:SNOUT_EXPECT_MSI -ErrorAction SilentlyContinue
   ```

Record Windows version, both release versions, installation paths, probe results,
scope, update outcome, registered versions, and uninstall outcomes. Native
validation remains pending until these steps have been run on Windows.
