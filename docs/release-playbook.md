# Release playbook for storytold apps

How every crafting app (DesignCraft, DrawCraft, EffectCraft, FilmCraft, LightCraft,
PhotoCraft, PrintCraft; later ArtCraft and ArtCraftX) builds signed release binaries in CI.
PhotoCraft is the reference implementation: copy its `.github/workflows/release.yml`, `packaging/`
and `xtask version` code into your repo, then rename. Repos never share code (see
`../craftrules/README.md`): each app keeps and evolves its own copy. PhotoCraft-specific details
live in its `docs/releasing.md`.

This playbook is meant to move to `craftrules/release/playbook.md` as the canonical shared copy.
Until then, this file is the source.

Agents: follow this document step by step, and keep each app's setup identical to it except for
the names in §9. If you improve the recipe in one app, update the playbook too.

**Naming.** Display names follow `{Function}Craft`, PascalCase with a capital "C": **ArtCraft,
ArtCraftX, DesignCraft, DrawCraft, EffectCraft, FilmCraft, LightCraft, PhotoCraft, PrintCraft**.
Use them for:
- release names (`PhotoCraft v0.2.0`);
- the macOS bundle (`PhotoCraft.app`, `CFBundleName`/`CFBundleDisplayName`), the DMG volume name;
- the Windows product name, Start Menu shortcut and installer title;
- `.desktop` `Name=`, AppStream `<name>`;
- window titles and About dialogs.

Machine names stay lowercase: cargo packages and binaries (`photocraft`), file names
(`photocraft-0.2.0-macos-universal.dmg`), repo names, and ids (`ai.storyteller.photocraft`).
Below, `<App>` is the display name and `<app>` the lowercase name.

## 1. What a release produces

Every push to the `release` branch builds everything below and attaches it to a **draft** GitHub
Release named `<App> v<version>`, tagged `v<version>`, plus `SHA256SUMS.txt`. A human reviews and
publishes the draft.

| Platform | Artifacts | Signing |
|---|---|---|
| macOS | Universal (arm64 + x86_64) `<App>.app` in a `.dmg`; universal CLI `.zip` if the app has one | Developer ID codesign (hardened runtime) + Apple notarization + stapling |
| Windows | x64 and x86 `.msi` installers + portable `.zip`s | Authenticode (signtool, timestamped), or Azure Trusted Signing |
| Linux | x86_64 (+ aarch64 when cheap): `.AppImage`, `.deb`, `.rpm`, `.tar.gz`; a Flatpak manifest | None needed (checksums in `SHA256SUMS.txt`) |
| Web | `<app>-web-<version>.zip` (wasm + generated glue + `index.html`) | None |

Why these Linux formats:
- **AppImage** runs on almost any distro without installing.
- **.deb** covers Debian, Ubuntu, Mint and Pop!_OS, the largest desktop share.
- **.rpm** covers Fedora, openSUSE and RHEL.
- **.tar.gz** is for everyone else and for scripts.
- **Flatpak** (Flathub) is the best long-term channel for sandboxed, auto-updating installs. We keep a manifest ready and submit to Flathub separately, because Flathub builds from source itself.

Build Linux on **ubuntu-22.04**. Its glibc 2.35 is old enough for current Debian, Ubuntu LTS and
Fedora.

## 2. Triggers

```yaml
on:
  push:
    branches: [release]
  workflow_dispatch:
    inputs:
      version:
        description: "Override version for a test build (e.g. 0.2.0-rc.1)"
        required: false
concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false
```

Every job that needs secrets declares `environment: release` (see §3). The job that creates the
release also declares `permissions: contents: write`.

## 3. Secrets and branch protection (security model)

**Never store signing secrets as org-level or repo-level Actions secrets.** GitHub can't restrict
those to a branch: anyone with write access could push a workflow to any branch and read them.
Instead:

1. **A `release` environment in every repo**, with deployment branches limited to exactly
   `release`. The signing secrets live there as *environment secrets*, so only workflow runs on
   `refs/heads/release` can read them.
2. **A ruleset on `release` in every repo**:
   - target `refs/heads/release`;
   - rules: restrict creations, restrict updates, restrict deletions, block force pushes;
   - bypass list: the `release-managers` team (echelon, echelon-robot) plus the
     *Organization admin* role. Rulesets can't list individual users, hence the team.

   The result is that only those people can start a run that sees the secrets.
3. **Canonical copy:** `~/secrets/storytold/` on the release manager's machine (dir 700, files
   600). Binary items (`.p12`, `.pfx`) are stored decoded next to their base64 form. A sync script
   pushes them into each repo's `release` environment with `gh secret set --env release -R storytold/<repo>`.
   Adding a repo = create the environment + ruleset, then run the sync.
4. **Public repos have world-readable logs.** Never `echo`, `cat` or `set -x` around secrets.
   Write them to files with `umask 077` and delete them in an `always()` step. Fork PRs never
   receive secrets, and must not be given them via `pull_request_target`.
5. Every secret is **optional** in the workflow. When one is missing, the job builds unsigned and
   emits `::warning::`, so the pipeline also runs in forks and before secrets exist.

Secret names (identical in every repo):

| Secret | Use |
|---|---|
| `APPLE_CERTIFICATE` | base64 `.p12`: "Developer ID Application: Learning Machines LLC (DJ6XS33FX8)" |
| `APPLE_CERTIFICATE_PASSWORD` | password of that `.p12` |
| `KEYCHAIN_PASSWORD` | throwaway password for the temporary CI keychain |
| `APPLE_ID` | Apple ID used for notarization |
| `APPLE_PASSWORD` | app-specific password for `notarytool` |
| `APPLE_TEAM_ID` | `DJ6XS33FX8` |
| `WINDOWS_CERTIFICATE` / `WINDOWS_CERTIFICATE_PASSWORD` | base64 `.pfx` for signtool (if we use a PFX) |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Azure Trusted Signing (if we use it instead of a PFX) |

Status (2026-10-01): the Apple secrets exist as storytold org secrets used by artcraft. They are
being exported into `~/secrets/storytold/` and moved into per-repo `release` environments. Windows
signing material is still being identified, because artcraft never signed its Windows builds.

## 4. Versioning

- **Source of truth:** `[workspace.package] version` in the root `Cargo.toml`. Packaging metadata
  must not hardcode it.
- **Bumping:** `cargo xtask version` prints the version, and `cargo xtask version set X.Y.Z[-pre]`
  rewrites it and refreshes `Cargo.lock`. Cut a release by bumping on `main`, then merging `main`
  into `release`.
- **In the app:** read the version from `CARGO_PKG_VERSION`. A `build.rs` (or `option_env!`) adds
  the short commit and build date from CI env vars (e.g. `<APP>_BUILD_SHA`), with fallbacks so
  local and wasm builds still compile without git. Show it in the About dialog, `--version`, and
  the window title if appropriate.
- **Overrides:** the release workflow reads the version from `Cargo.toml`. The `workflow_dispatch`
  `version` input overrides it for test builds; apply it to the build too, not just the release name.
- **Naming:** the tag is `v<version>`; the release name is `<App> v<version>`; files are named
  `<app>-<version>-<platform>-<arch>.<ext>`.

## 5. macOS recipe (universal, signed, notarized)

1. Build both architectures and merge them:
   ```sh
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   cargo build --release -p <app> --target aarch64-apple-darwin
   cargo build --release -p <app> --target x86_64-apple-darwin
   lipo -create -output <App> target/{aarch64,x86_64}-apple-darwin/release/<app>
   ```
2. Assemble `<App>.app/Contents/{MacOS/<App>, Resources/<App>.icns, Info.plist}`. Info.plist keys:
   - `CFBundleIdentifier` = `ai.storyteller.<app>`
   - `CFBundleShortVersionString` / `CFBundleVersion`
   - `CFBundleIconFile`
   - `LSMinimumSystemVersion`: what wgpu/Metal needs
   - `NSHighResolutionCapable`
   - `CFBundleDocumentTypes` for the app's file types
3. Import the certificate into a temporary keychain. ArtCraft's flow works:
   - `security create-keychain` → `default-keychain -s` → `unlock-keychain` → `set-keychain-settings -t 3600 -u`
   - `security import cert.p12 -P … -T /usr/bin/codesign`
   - `security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD"`
   - add the keychain to the search list.

   Delete the keychain and `.p12` in an `always()` step.
4. Sign inside-out (nested code first, then the bundle) with `--options runtime --timestamp` and
   minimal entitlements. Avoid `--deep` on the final signature.
5. Make the DMG (`hdiutil create -format UDZO`, ideally with an `/Applications` symlink), then sign it.
6. Notarize: `xcrun notarytool submit <dmg> --apple-id … --password … --team-id … --wait`, then
   `xcrun stapler staple` the DMG (and the `.app` before packaging if it ships elsewhere).
7. Verify: `codesign --verify --strict --verbose=2`, `spctl -a -vvv -t install <dmg>`,
   `spctl -a -vvv <App>.app`.
8. Locally, run the same script with `codesign -s -` (ad-hoc) and no notarization to test bundling.

## 6. Windows recipe (x64 + x86, signed MSI)

1. Targets: `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc` on `windows-latest`.
2. Set `RUSTFLAGS=-C target-feature=+crt-static` so users don't need the VC++ redistributable.
3. Embed the icon and version info in the exe through a `build.rs` using `winresource` or
   `embed-resource`, gated to Windows targets so macOS, Linux and wasm builds are unaffected.
4. Build the MSI with WiX (v4/v5 dotnet tool, or cargo-wix): per-machine install, Start Menu
   shortcut, file associations, upgrade code fixed per app (generate once, never change), product
   code per version. Also zip the exe as a portable build.
5. Sign the exe before packaging and the MSI after, using one small script that picks the method:
   - PFX: `signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f cert.pfx /p …`
   - or Azure Trusted Signing (the `azure/trusted-signing-action`, or `signtool` with the dlib).

   Verify with `signtool verify /pa`.

## 7. Linux recipe

1. Run on `ubuntu-22.04` (and `ubuntu-24.04-arm` for aarch64). Install the egui/winit/wgpu build deps:
   `libxkbcommon-dev libwayland-dev libx11-dev libxrandr-dev libxi-dev libgl1-mesa-dev libgtk-3-dev`.
2. Shared assets in `packaging/linux/`:
   - `<app>.desktop`, with `MimeType=` for the app's formats;
   - `ai.storyteller.<app>.metainfo.xml` (AppStream);
   - hicolor PNG icons at 16–512 px plus a scalable SVG.
3. `.deb`: `cargo-deb` (or nfpm), declaring runtime deps such as `libxkbcommon0`, `libwayland-client0`, `libgtk-3-0` and `libvulkan1 | mesa-vulkan-drivers`.
4. `.rpm`: `cargo-generate-rpm` (or nfpm) with the equivalent deps.
5. `.AppImage`: build an AppDir (binary + desktop + icon + AppRun), then run `appimagetool`, or `linuxdeploy` to bundle libraries.
6. Flatpak: keep a manifest at `packaging/linux/flatpak/ai.storyteller.<App>.yml` (freedesktop runtime, `--socket=wayland --socket=fallback-x11 --device=dri --filesystem=home` or portals). Build it in CI only if it's quick; otherwise validate it and submit to Flathub separately.

## 8. Web/WASM recipe

1. `rustup target add wasm32-unknown-unknown`, install `trunk`, then run `trunk build --release` in the
   web app dir. `Trunk.toml` must set `public_url = "./"` so the bundle works from any path.
2. Zip the dist dir. The README in `packaging/web/` covers:
   - serving `.wasm` as `application/wasm` with gzip/brotli;
   - the iframe embed snippet;
   - URL flags for the WebGPU/WebGL2/CPU fallbacks;
   - the rule that no hand-written JS is involved.
3. A later job can deploy the zip to the website. The release only attaches it.

## 9. Adopting this in another app (checklist)

1. Copy from photocraft:
   - `.github/workflows/release.yml`
   - `packaging/{macos,windows,linux,web}/`
   - the `xtask version` command
   - the version/build-info `build.rs` and About/`--version` wiring
2. Rename everywhere: app name, binary/package names, `ai.storyteller.<app>` bundle id,
   `.desktop`/metainfo ids, file associations, the WiX upgrade code (new GUID), icons, release name.
3. Make sure the app's workspace version is the single source (§4).
4. In GitHub (an org admin does this, by API or the web UI):
   - create the `release` environment with a deployment-branch rule of `release` only;
   - add the ruleset from §3.2 (`release-managers` team + Organization admin bypass);
   - run the secret sync from `~/secrets/storytold/` for this repo.
5. Test with `workflow_dispatch` and `version: X.Y.Z-rc.1` from the `release` branch. Check every
   artifact installs and launches on each OS: Gatekeeper accepts the DMG with no warning, and
   SmartScreen shows the publisher.
6. Document anything app-specific in that app's `docs/releasing.md`. Improvements to the recipe
   itself go back into this playbook.

## 10. Copy, don't depend

Crafting apps never share code (craftrules README). There is deliberately **no** shared reusable
workflow or packaging library: each app copies PhotoCraft's files and owns them. To make copying
easy, the reference implementation keeps every app-specific value (display name, package, bundle
id, web dir, WiX upgrade GUID, file types) in one clearly marked place at the top of the workflow
and scripts. When an app finds a better recipe, it updates this playbook (and a learning in
craftrules) so the others can adopt the idea in their own copies.

## 11. Prompt for an app's agent

Paste this into the agent working in another app's repo (replace `<App>` / `<app>`):

> Set up signed release builds for `<App>` (display name in PascalCase, e.g. DrawCraft; lowercase
> `<app>` for packages, binaries, files and ids) following `docs/release-playbook.md` in the photocraft
> repo (`storytold/photocraft`). Read it fully first, then mirror photocraft's implementation:
> copy `.github/workflows/release.yml`, `packaging/{macos,windows,linux,web}/`, the `cargo xtask version`
> command, and the version/build-info `build.rs` + About/`--version` wiring. Rename everything
> per §9: app name, package, `ai.storyteller.<app>` bundle id, `.desktop`/metainfo ids, file
> associations, and a NEW WiX upgrade GUID. Keep the secret names, triggers, artifact naming
> (`<app>-<version>-<platform>-<arch>.<ext>`) and the `environment: release` gating exactly as
> written. Make the workspace version the single source of truth. Verify locally what you can:
> build the app; run the macOS packaging script with an ad-hoc signature; lint the workflow with
> actionlint. Don't create GitHub environments, rulesets or secrets yourself: an org admin does
> that (§9.4). Don't commit or push unless asked. Report what you copied, what you renamed, and
> anything app-specific that needed a change, and propose playbook edits if the recipe was wrong.

## 12. Non-Rust or non-eframe apps

The security model (§3), versioning rules (§4), naming, triggers and artifact matrix (§1) still
apply. Swap only the build commands: for example, Tauri apps can keep `tauri-action` for bundling
but must still read secrets only from the `release` environment.
