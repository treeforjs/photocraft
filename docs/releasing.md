# Releasing PhotoCraft

Every push to the `release` branch runs `.github/workflows/release.yml`. The workflow builds
Windows and Linux packages, signs their checksum list, then creates or updates a **draft**
GitHub Release named `PhotoCraft v<version>`. Nobody sees a draft until a maintainer publishes it.
The release pipeline has no macOS, FreeBSD, web or Flatpak build jobs.

This document is the release recipe for this repository (`release-playbook.md` only points here).
User-facing names say **PhotoCraft**. Files, binaries
and ids stay lowercase (`photocraft-<version>-<platform>-<arch>.<ext>`, `io.github.treeforjs.photocraft`).

## Cutting a release

1. **Bump the version** on `main`. The only place it lives is `[workspace.package] version`
   in the root `Cargo.toml`:

   ```sh
   cargo xtask version                 # prints the current version, e.g. 0.1.0
   cargo xtask version set 0.2.0       # or 0.2.0-rc.1; updates Cargo.toml and Cargo.lock
   ```

   Commit the change (`Cargo.toml` + `Cargo.lock`) through the normal review flow.
2. **Merge `main` into `release`** (or fast-forward it) and push. The workflow starts by itself.
3. **Wait for the draft.** After the Windows/Linux builds and Ubuntu smoke checks pass, the
   Releases page has a draft `PhotoCraft v0.2.0`, tagged `v0.2.0` on the pushed commit, with
   every artifact, `SHA256SUMS.txt` and `SHA256SUMS.txt.sig`. The notes describe the available
   Windows/Linux packages; add the changes users should know about before publishing.
4. **Check it.** Test the Windows portable ZIP and Linux AppImage in Craftworks, and inspect
   the job summaries. Ed25519 checksum signing is required. Windows Authenticode certificates
   are optional; warnings about those certificates do not affect Craftworks verification.
5. **Add the scorecard deltas** to the draft's notes: what moved in
   [`docs/scorecard.md`](scorecard.md) since the previous release. Compare the summary table
   and the performance table of both tags (`git diff v<previous> v<this> -- docs/scorecard.md`)
   and list each area's done / partial / missing change, the scenarios that newly meet their
   budget or got slower, corpus floors raised, and the change in settings that do nothing.
   Quote numbers with the baseline's machine and load average.
6. **Publish** the draft in the GitHub UI. Publishing creates the `v0.2.0` tag. Versions with a
   pre-release suffix (`-rc.1`) are marked as pre-releases.

Pushing to `release` again before you publish rebuilds the same draft and replaces its assets.
After the draft is published, the workflow refuses to touch that version again, so bump it first.

**Test runs:** *Actions → Release → Run workflow* runs the whole pipeline by hand. The optional
`version` input (such as `0.2.0-rc.1`) overrides `Cargo.toml` for that run only. The jobs apply
it with `cargo xtask version set` before building, so the binaries report it too. The run still
needs the `release` environment, which only the `release` branch can use, so pick that branch
in the dialog. A manual run on another branch never creates or updates a release; its builds
remain subject to the environment's deployment branch policy.

## What gets built

| Platform | Artifacts | Built on |
|---|---|---|
| Windows 10+ x64 | `photocraft-<v>-windows-x64.msi`, `photocraft-<v>-windows-x64-portable.zip` | `windows-latest` |
| Windows 10+ x86 (32-bit) | `photocraft-<v>-windows-x86.msi`, `photocraft-<v>-windows-x86-portable.zip` | `windows-latest` |
| Windows ARM64 | `photocraft-<v>-windows-arm64.msi`, `photocraft-<v>-windows-arm64-portable.zip` | Cross-compiled on `windows-latest` |
| Linux x86_64 | `photocraft-<v>-linux-x86_64.{AppImage,AppImage.zsync,deb,rpm,tar.gz}` | `ubuntu-22.04` |
| Linux aarch64 | `photocraft-<v>-linux-aarch64.{AppImage,AppImage.zsync,deb,rpm,tar.gz}` | `ubuntu-22.04-arm` |

Every binary reports its version, the commit and the build date: `photocraft --version`,
`photocraft-cli --version`, and *Help › About PhotoCraft*. CI sets `PHOTOCRAFT_BUILD_SHA` and
`PHOTOCRAFT_BUILD_DATE`, and `crates/engine/src/build_info.rs` reads them at compile time. A plain
`cargo build` doesn't set them and reports `0.2.0 (dev build)`.

Each Windows/Linux build checks out [craft-fonts](https://github.com/storytold/craft-fonts)
at the commit in `CRAFT_FONTS_REF` (top of `release.yml`) and builds with `CRAFT_FONTS_DIR` and
`CRAFT_FONTS_REQUIRED=1`, so releases embed its Japanese fonts and fail rather than ship without
them. The packages carry each font's licence as `OFL-<family>.txt`. Bump the pin deliberately,
together with the one in `ci.yml`. Build option: `docs/development.md` → Fonts.

### Windows

`packaging/windows/package.ps1 -Arch x64|x86|arm64` builds with `-C target-feature=+crt-static`.
The static C runtime means neither the MSI nor the portable zip needs the Visual C++
redistributable, which matters for a standalone installer and costs only about 100 KB. The flag
goes in `CARGO_TARGET_<TRIPLE>_RUSTFLAGS`, so host build scripts aren't affected.

- `apps/photocraft/build.rs` and `apps/photocraft-cli/build.rs` embed the icon
  (`assets/app-icon/photocraft.ico`) and VERSIONINFO with the `winresource` crate
  (build dependency `0.1`). They only do this when targeting Windows. Elsewhere each
  script is a no-op, even if `PHOTOCRAFT_REQUIRE_WINRES` is set, and the web build
  doesn't touch that crate. Both set ProductName to PhotoCraft, CompanyName to
  Craft contributors and the same copyright line. The GUI description is
  "PhotoCraft image editor" (`photocraft.exe`). The CLI description is
  "PhotoCraft command-line interface", with OriginalFilename `photocraft-cli.exe`
  and InternalName `photocraft-cli`. Version strings are the Cargo package version,
  so a pre-release tag is kept; the numeric file and product versions are
  winresource's `major.minor.patch.0`. A missing resource compiler warns, so an
  optional cross-compile still links. `package.ps1` sets `PHOTOCRAFT_REQUIRE_WINRES=1`.
  The CLI build script then returns an error naming `rc.exe` or `windres` and does
  not panic.
- Release builds use the GUI subsystem, so Start Menu launches don't open a console window.
  The CLI stays console subsystem 3 so its output reaches the terminal.
- `photocraft.wxs` (WiX v5) is a per-machine install into Program Files with an advertised
  Start Menu shortcut. PhotoCraft becomes the default app for `.pcraft` and is listed under
  "Open with" for PSD/PSB and image files. It also registers App Paths (Win+R `photocraft`).
  The MSI version is the numeric `X.Y.Z`, because MSI has no pre-release field. Same-version
  upgrades are allowed so that release candidates replace each other.
- Double-clicking the MSI opens a setup wizard (WixUI_InstallDir without the licence page):
  Welcome, install folder (remembered for upgrades in `HKLM\Software\PhotoCraft\InstallDir`),
  Ready, a progress page, and a Finish page that says where to find the app, with a ticked "Launch
  PhotoCraft" box. The pages name the full version (`photocraft.en-us.wxl`). `package.ps1` draws
  the wizard's banner and side bitmaps from the app icon, so no WiX stock art ships. `msiexec /qn`
  still installs silently. Building needs the WiX UI and Util extensions:
  `wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`.
- Shortcut icon identifiers keep the executable's `.exe` extension: MSI uses the identifier
  as the cached icon filename, and an extensionless filename can render as a blank document
  icon. `packaging/windows/check-icons.ps1` checks the references and extensions in CI;
  `package.ps1` also validates the built MSI with ICE50 before signing it.
- **Portable zip:** it ships `packaging/windows/portable.txt` beside `photocraft.exe`. That
  marker (or a `PhotoCraft.portable` file) switches on portable mode: preferences, presets,
  recovery autosaves and the GPU startup marker go to `PhotoCraftData\` next to the exe instead
  of `%APPDATA%\Photocraft`. If that folder isn't writable the app warns and uses `%APPDATA%`.
  The MSI has no marker. The logic is in `apps/photocraft/src/app_dirs.rs` and works the same
  on macOS and Linux.
- **Signing:** `packaging/windows/sign.ps1` signs both `.exe` files and then the `.msi` with
  `signtool`, using SHA-256 and an RFC 3161 timestamp. It uses whichever material is present:
  1. a `.pfx` file (`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD`), timestamped by
     DigiCert, or
  2. Azure Trusted Signing (`AZURE_*`): the script downloads the
     `Microsoft.Trusted.Signing.Client` dlib and timestamps with Microsoft's server.

  The script is the single place to change when the Windows signing setup changes.

Locally on Windows: `dotnet tool install -g wix --version 5.0.2`,
`wix extension add -g WixToolset.UI.wixext/5.0.2 WixToolset.Util.wixext/5.0.2`, then
`pwsh packaging/windows/package.ps1 -Arch x64`.

### Linux

`packaging/linux/package.sh` stages one FHS tree and makes every format from it. The tree
holds both binaries, `io.github.treeforjs.photocraft.desktop`, hicolor icons from 16 px to 512 px
plus a scalable SVG, AppStream metainfo, and a shared-mime-info file for `.pcraft`, `.psb` and
`.qoi`.

The staging tree has explicit system-readable permissions even when built from a
private checkout. `.deb` dependencies use the greatest required GLIBC symbol
version from both binaries (with the official 2.35 floor), including when using
`--skip-build`. `packaging/linux/package-test.sh` checks staging permissions and
the generated Debian metadata without compiling the application.

Before publishing, the release workflow installs the x86_64 `.deb` in Ubuntu
24.04 and 26.04 containers. `packaging/linux/smoke-test.sh` exercises window
rendering, save/reopen and the CLI under Xvfb as an ordinary user, using Mesa
software rendering, with both automatic backend selection and forced OpenGL.
Screenshots and logs are retained as CI artifacts. See the
[Ubuntu guide](ubuntu.md) for installation and local smoke-test commands.

The Linux x86_64 job also runs `packaging/linux/check-window-frame.sh` in a private nested
Weston/Xvfb desktop. It forces Wayland and captures the full native window, including the
standard minimize, maximize and close controls, as `window-frame-linux-x86_64`. Review that
artifact before publishing. The check uses temporary settings and never opens the user's
desktop or document library. Its prerequisites are Weston, Xvfb, scrot, Mesa EGL/GL and
`fonts-cantarell`; its two arguments are the built executable and an output directory.

Linux packages include the Cantarell fallback font's OFL notice under
`share/licenses/photocraft/OFL-Cantarell.txt`, including inside the AppImage. The font comes
from the locked `sctk-adwaita` dependency; no new font file is stored in this repository.
The same directory retains `NOTICE`, including the window frame dependency's MIT license.

Why these formats:

- **AppImage** runs on any distribution without installing anything. The
  download-and-go option and the fallback for distros the packages don't cover. It's built
  with the maintained `AppImage/appimagetool`, whose static runtime doesn't need libfuse2.
  Each AppImage embeds update information (`gh-releases-zsync|…|latest|…`, #349) and ships
  with a `.zsync` beside it, so AppImageUpdate, AppImageLauncher and similar tools can find
  the next release and download only the blocks that changed.
  Its `AppRun` (`packaging/linux/AppRun`) also integrates with the desktop on launch: on
  Wayland the dock icon comes from the compositor resolving the window's app_id against an
  *installed* `.desktop` file, which an AppImage alone never provides — without this every
  AppImage run shows the generic Wayland logo (#593). Best effort: it installs
  `~/.local/share/applications/io.github.treeforjs.photocraft.desktop` with `Exec` rewritten to
  the AppImage's path (`TryExec` dropped, the binary isn't on the host PATH) and the
  hicolor icons beside it, rewriting only when the content changed, and any failure leaves
  the launch untouched. `PHOTOCRAFT_NO_DESKTOP_INTEGRATION=1` opts out; to uninstall, `rm
  ~/.local/share/applications/io.github.treeforjs.photocraft.desktop` and `find
  ~/.local/share/icons/hicolor -name 'io.github.treeforjs.photocraft*' -delete`. The script is
  covered by `packaging/linux/apprun-test.sh` in the packaging-lint workflow.
- **.deb** covers Debian, Ubuntu, Mint, Pop!_OS and elementary. **.rpm** covers Fedora, RHEL
  and its clones, and openSUSE. Both integrate with the menu, MIME and icon caches (the
  `postinst.sh` hook) and uninstall cleanly. Both are built by
  [nfpm](https://nfpm.goreleaser.com/): one maintained, dependency-free tool and one config
  (`nfpm.yaml`). It doesn't need cargo-deb and cargo-generate-rpm metadata in our crates, and
  it can build both formats on Ubuntu.
- **.tar.gz** is for people who manage their own `/opt` or `~/.local`.
- **Flatpak bundle** is available as a separate local packaging option, outside the release
  workflow. `packaging/linux/flatpak-bundle.sh` repackages the Linux `.tar.gz` using
  `packaging/linux/flatpak/io.github.treeforjs.photocraft.bundle.yml`. See that script for the
  freedesktop runtime and build requirements. It is not needed by Craftworks.
- **Flathub**: `packaging/linux/flatpak/io.github.treeforjs.photocraft.yml` builds from source and
  is ready for a Flathub submission (it keeps the same runtime and `finish-args` as the bundle
  manifest, which packaging-lint checks). CI doesn't build it. A real build needs vendored crate
  sources (`cargo-sources.json` from `flatpak-cargo-generator.py`) and adds 20+ minutes per
  architecture. Build it by hand with the commands in the manifest's header.

Official Linux binaries are built on Ubuntu 22.04 and need **glibc ≥ 2.35**: Ubuntu 22.04+,
Debian 12+, Fedora 36+, RHEL 10, openSUSE Tumbleweed. They link only glibc and libgcc_s. X11,
Wayland, xkbcommon, Vulkan and EGL are loaded at runtime from the system, which is also where
the GPU driver has to come from. That's why the .deb and .rpm declare them as dependencies
(the full list, and why each is there, is in `packaging/linux/nfpm.yaml`) and the AppImage
doesn't bundle them. The Flatpak gets them from the freedesktop runtime. Because the AppImage
and the tarball can't declare dependencies, `photocraft` checks for the libraries its session
(X11 or Wayland) needs before it opens a window (`apps/photocraft/src/linux_libs.rs`) and, if
one is missing, prints the package to install and exits with status 1 instead of crashing.
`PHOTOCRAFT_SKIP_LIB_CHECK=1` skips the check.

Locally (on Linux): install [nfpm](https://nfpm.goreleaser.com/install/), then
`packaging/linux/package.sh` (or `--formats "deb tar"`).

### Other local packaging

Existing scripts under `packaging/macos/`, `packaging/freebsd/` and `packaging/web/` remain
available for local development, but do not run when cutting a Windows/Linux release.
Their separate CI workflows are unchanged.

## Secrets

The **`release` environment** (*Settings → Environments → release*) holds the signing
secrets. Restrict its deployment branches to `release`. The initial version job validates the
Ed25519 key pair before the Windows/Linux builds start. A missing, invalid or mismatched
`UPDATE_SIGNING_KEY` or `CRAFT_UPDATE_PUBLIC_KEY` stops the release; Windows Authenticode
certificates remain optional. Linux needs no certificate or private key in its build job.

| Secret | Used for |
|---|---|
| `WINDOWS_CERTIFICATE` | base64 `.pfx` code-signing certificate |
| `WINDOWS_CERTIFICATE_PASSWORD` | password for that `.pfx` |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET` | service principal for Azure Trusted Signing (an alternative to the `.pfx`) |
| `AZURE_SIGNING_ENDPOINT` | e.g. `https://eus.codesigning.azure.net` |
| `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` | Trusted Signing account and certificate profile names |
| `UPDATE_SIGNING_KEY` | Ed25519 private key (PEM) that signs `SHA256SUMS.txt` for in-app updates (see below) |

`GITHUB_TOKEN` creates the release. Only the final job gets `contents: write`.

### In-app updates

Help › Check for Updates… (and the daily background check, Preferences › General) uses
[`craft-update`](https://github.com/treeforjs/craftworks/tree/main/crates/craft-update): it
finds the newest release, downloads the installer matching how PhotoCraft was installed (MSI,
AppImage, deb, rpm), verifies it against `SHA256SUMS.txt` and that list's Ed25519 signature,
and installs it when the app quits. The one-time key setup is in craftworks'
[`docs/updates.md`](https://github.com/treeforjs/craftworks/blob/main/docs/updates.md):

- `UPDATE_SIGNING_KEY` (secret, `release` environment): the draft-release job signs
  `SHA256SUMS.txt` into `SHA256SUMS.txt.sig` and uploads it with the other files.
- `CRAFT_UPDATE_PUBLIC_KEY` (a *repository* variable, *Settings → Secrets and variables →
  Actions → Variables*, not secret): the Windows and Linux build jobs embed it. It must be
  a repository variable rather than a `release`-environment one because the Linux job runs
  without that environment.

The public variable accepts base64 Ed25519 SubjectPublicKeyInfo DER (44 bytes) or raw public
key bytes (32 bytes). Both forms must match the private key. Use the same public key in
Craftworks so it can verify these packages. The workflow validates the pair again, signs the
complete checksum list and verifies the signature before uploading any release assets.

Generate and store production keys outside the repository; see the linked one-time setup.
`bash packaging/check-update-key-test.sh` exercises validation and signing with temporary
throwaway keys, including malformed keys, mismatches and tampered checksum files.

## Icons

`assets/app-icon/photocraft.svg` is the canonical icon: the original owner's hand drawing of a
nine-tailed kitsune, vectorised (MIT OR Apache-2.0, see `LICENSE.txt` there; palette and
geometry in `README.md`). `packaging/icons.sh` regenerates the 1024 px PNG, the `.icns`, the `.ico` (packed by
`cargo xtask ico`) and the hicolor PNGs from it. It needs `resvg`, plus `iconutil` on macOS.
The outputs are committed, so packaging never needs those tools.

## Checks

`.github/workflows/packaging-lint.yml` runs in seconds on any change to `packaging/`, the
workflows or the icons. It runs actionlint, shellcheck, a PowerShell parse, xmllint,
`desktop-file-validate`, `appstreamcli validate`, and a YAML check that the two Flatpak
manifests agree on the runtime and permissions.

## Installed identity migration

The desktop and package identity is `io.github.treeforjs.photocraft`. The Windows MSI UpgradeCode remains unchanged, so existing MSI installations upgrade in place. The preference and document paths are unchanged. Linux package upgrades replace the old desktop resources. AppImage startup removes a legacy shortcut only when its name, icon and executable identify this exact AppImage; it preserves shortcuts to other installations. Existing Flatpak installations use a different application ID and need a separate install of the new ID; their data is not removed automatically.

## Generated export files

Render Video writes beneath the selected output folder and refuses existing destinations. Image Assets accepts relative subfolders such as `icons/logo.png`, rejects parent paths, device names and escaping symlinks, and never overwrites an unrelated file. It can refresh an output during the same session only while the existing bytes still match that session's last generated image. Outputs from earlier sessions, or files edited outside PhotoCraft, need a fresh output folder or explicit user removal before regeneration. A failed asset export after saving appears as a warning; the saved document remains saved.
