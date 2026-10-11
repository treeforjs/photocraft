# PhotoCraft on Ubuntu

PhotoCraft is a native desktop application. Ubuntu **24.04 LTS and 26.04 LTS on
x86_64 (Intel/AMD 64-bit)** are covered by the release package smoke-test matrix.
The test installs the `.deb`, opens an X11 window with Mesa software rendering,
creates and saves a document, reopens it, captures the window, and checks the same
file with the headless CLI. Both automatic backend selection and the OpenGL
fallback are exercised. Physical GPUs, pen tablets and a GNOME Wayland session
still need testing on the target machine.

During the audit, the installed package also passed the document round trip and
screenshot check on both Ubuntu versions under native headless Weston Wayland.
These X11 and Wayland checks ran as an ordinary user with Mesa software graphics.

## Install a package

Download `photocraft-<version>-linux-x86_64.deb` and `SHA256SUMS.txt` from the same
[release](https://github.com/storytold/photocraft/releases). From the download
directory, replace `<version>` with the downloaded version:

```sh
sha256sum --check --ignore-missing SHA256SUMS.txt
sudo apt update
sudo apt install ./photocraft-<version>-linux-x86_64.deb
photocraft
```

Use `apt install ./...` so Ubuntu also installs the runtime libraries. The package
adds PhotoCraft to the application menu, registers image file associations, and
installs `photocraft-cli`. Install a newer `.deb` the same way to update; remove
the package with `sudo apt remove photocraft`. Documents and user preferences are
kept separately from the package.

Published upstream packages can lag behind a branch in a fork. To use changes
from this checkout before a release includes them, build from source below.

The AppImage is an alternative when you do not want a system-wide install:

```sh
chmod +x photocraft-<version>-linux-x86_64.AppImage
./photocraft-<version>-linux-x86_64.AppImage
```

If the system cannot mount an AppImage, run it with
`APPIMAGE_EXTRACT_AND_RUN=1 ./photocraft-<version>-linux-x86_64.AppImage`.
Keep it in a stable location: its launcher refers to that path. A minimal Ubuntu
installation may need the libraries named by PhotoCraft's startup message.

## Build this checkout

Run these commands on Ubuntu 24.04 or 26.04:

```sh
sudo apt update
sudo apt install build-essential pkg-config curl ca-certificates git \
  libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libx11-dev \
  libxrandr-dev libxi-dev libxcursor1 libgl1-mesa-dev libvulkan1 libegl1 \
  mesa-vulkan-drivers libegl-mesa0 xdg-desktop-portal
```

Install Rust using [rustup](https://rustup.rs/), then use Rust stable **1.95 or
newer**. Ubuntu's distribution `rustc` package can be older than this workspace
requires. In the repository checkout, with the desired branch selected:

```sh
rustup update stable
cargo +stable build --release --locked -p photocraft -p photocraft-cli --features heif
./target/release/photocraft
./target/release/photocraft-cli --version
```

The `heif` feature enables opening HEIC photos. System fonts work without an
additional font repository; install `fonts-noto-cjk` for CJK text if needed.
Full GNOME Ubuntu installations include a desktop portal backend; on a minimal
GNOME installation also install `xdg-desktop-portal-gnome` for native file dialogs.

To make a `.deb` and portable tarball, install [nFPM](https://nfpm.goreleaser.com/install/)
and, after the build above, run:

```sh
packaging/linux/package.sh --skip-build --formats "deb tar"
sudo apt install ./dist/release/photocraft-*-linux-x86_64.deb
```

Build on the oldest Ubuntu version where the result must run. A binary linked on
26.04 may require a newer glibc than 24.04 provides; official release binaries use
Ubuntu 22.04 to keep their glibc 2.35 baseline. The `.deb` dependency records the
glibc symbols required by the actual binaries, so an incompatible local build is
refused at installation time.

## Graphics hardware targets

The intended hardware configurations are:

| GPU | Ubuntu graphics stack |
|---|---|
| NVIDIA RTX PRO 4000 Blackwell and RTX PRO 2000 Blackwell workstation GPUs | Current Ubuntu-recommended NVIDIA desktop driver with **open kernel modules** |
| GeForce RTX 30, 40 and 50 series | Current Ubuntu-recommended NVIDIA desktop driver; RTX 50 Blackwell requires open kernel modules |
| Intel integrated graphics supported by Mesa | Ubuntu's kernel, firmware, Mesa Vulkan and OpenGL drivers |

These are hardware targets, not a claim of testing on each GPU. The automated
Ubuntu checks use Mesa software rendering under Xvfb. PhotoCraft uses Vulkan by
default on Linux and also supports the OpenGL fallback. Help → System Info shows
the adapter, active backend and whether the canvas uses GPU or CPU rendering.

For NVIDIA, first install Ubuntu's available system and kernel updates and
restart. Follow Ubuntu's [NVIDIA driver installation guide](https://ubuntu.com/desktop/docs/en/latest/how-to/graphics/install-nvidia-drivers/):

```sh
sudo apt install ubuntu-drivers-common
sudo ubuntu-drivers list
sudo ubuntu-drivers install
sudo reboot
```

Use the recommended desktop driver for the detected GPU. NVIDIA explicitly
[requires open kernel modules for Blackwell](https://download.nvidia.com/XFree86/Linux-x86_64/580.95.05/README/kernel_open.html);
the Ubuntu package normally has an `-open` suffix. Both workstation models appear
in NVIDIA's [supported GPU list](https://download.nvidia.com/XFree86/Linux-x86_64/595.84/README/supportedchips.html).
Use Ubuntu's current recommendation instead of pinning one driver version for
every supported Ubuntu release. After restarting, verify the loaded driver:

```sh
nvidia-smi
modinfo -F license nvidia
```

The open module reports `Dual MIT/GPL`. CUDA, a compute SDK and server-driver
packages are not required to run PhotoCraft.

For Intel integrated graphics, use the [Mesa graphics drivers](https://docs.mesa3d.org/systems.html)
and the [firmware needed by Intel ANV](https://docs.mesa3d.org/drivers/anv.html#runtime-dependencies):

```sh
sudo apt install libvulkan1 mesa-vulkan-drivers libegl1 libegl-mesa0 \
  libgl1 libgl1-mesa-dri linux-firmware
```

Recent Intel hardware can require a newer kernel. Ubuntu 26.04 includes a newer
kernel; on 24.04, use Ubuntu's supported [HWE kernel](https://ubuntu.com/kernel/docs/reference/hwe-kernels/)
when the installed kernel does not support the device:

```sh
sudo apt install --install-recommends linux-generic-hwe-24.04
sudo reboot
```

For either vendor, `sudo apt install vulkan-tools mesa-utils` provides
`vulkaninfo --summary` and `glxinfo -B` to identify the active graphics driver.
Compare these with PhotoCraft's System Info when diagnosing an unexpected
software adapter or the wrong GPU on a machine with multiple adapters.

## Display and graphics startup

Ubuntu normally uses Wayland. File → Open and the application menu work there.
Wayland drag and drop is unavailable in the current windowing library. Use File →
Open, or use Xwayland for drag and drop:

```sh
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET photocraft
```

This requires Xwayland and an existing `DISPLAY` in the desktop session. PhotoCraft
also selects Xwayland when a supported pen is attached because native Wayland pen
input is not implemented yet.

For graphics startup trouble, try `photocraft --safe-gpu`. This uses the CPU canvas
and a software window adapter where available; it still requires Mesa or a working
graphics driver. Logs are in `~/.config/photocraft/logs/photocraft.log`.

For a reproducible smoke check after a source build or package install:

```sh
sudo apt install xvfb xauth dbus-x11 jq
LIBGL_ALWAYS_SOFTWARE=1 xvfb-run -a dbus-run-session -- \
  bash packaging/linux/smoke-test.sh ./target/release/photocraft ./target/release/photocraft-cli
```

Pass `/usr/bin/photocraft /usr/bin/photocraft-cli` to check an installed package.
The script uses an isolated settings directory and temporary documents. Set
`PHOTOCRAFT_SMOKE_ARTIFACTS` to a directory to keep the screenshot and diagnostic
logs. This checks software rendering under X11; it does not replace checking the
real desktop, graphics driver and tablet hardware.
