# Windows graphics setup

Use PhotoCraft's **Windows x64** installer or portable ZIP on Windows 11 64-bit.
The graphics configurations below are intended hardware targets. Windows runtime
and physical GPU testing were not available during this audit; the driver
recommendations follow the vendors' supported-product lists.

## Choose the driver

| GPU | Recommended Windows driver |
|---|---|
| NVIDIA RTX PRO 4000 Blackwell and RTX PRO 2000 Blackwell | Current **NVIDIA RTX Enterprise Production Branch** for the exact workstation GPU and Windows 11 64-bit |
| GeForce RTX 30, 40 and 50 series | Current **NVIDIA Studio Driver** for creative work |
| Intel integrated graphics | PC manufacturer's supported graphics driver first; otherwise the Intel driver for the exact GPU generation |

For the RTX PRO workstation cards, use [NVIDIA's driver selector](https://www.nvidia.com/en-us/drivers/)
and check the supported products before downloading. NVIDIA's
[RTX Enterprise supported-product list](https://www.nvidia.com/en-us/drivers/details/279889/)
includes both Blackwell models. Match the desktop, SFF or laptop variant actually
installed in the machine. Follow the installer and restart Windows when it
finishes. Use the current applicable driver rather than pinning one version for
every GPU. The Linux requirement for open NVIDIA kernel modules does not apply
to Windows.

For GeForce, the [Studio driver supported-product list](https://www.nvidia.com/en-us/drivers/details/280101/)
covers RTX 30, 40 and 50 series. NVIDIA describes
[Studio and Game Ready drivers](https://www.nvidia.com/en-us/geforce/news/ces-2022-nvidia-community-qa/)
as serving creative application stability and new-game support respectively.
Game Ready is also an option when gaming is the main priority.

For Intel, follow the [manufacturer-driver guidance](https://www.intel.com/content/www/us/en/support/articles/000058295/graphics.html)
for a laptop or prebuilt workstation. Intel's
[driver selection guide](https://www.intel.com/content/www/us/en/support/articles/000090440/graphics.html)
and Driver & Support Assistant identify the applicable package. Arc/Core Ultra,
11th–14th generation integrated graphics and older graphics use different driver
families; a package for a newer generation is not a universal replacement.

Keep Windows updated. PhotoCraft uses the graphics APIs supplied by Windows and
the display driver; running it does not require the CUDA toolkit or Vulkan SDK.

## Check the active GPU

On a machine with Intel integrated graphics and an NVIDIA GPU, keep both display
drivers installed. Start PhotoCraft with its automatic GPU backend and inspect
**Help → System Info** for the active adapter, backend and driver.

If Windows chooses an unwanted GPU, go to **Settings → System → Display →
Graphics**, add or select `photocraft.exe`, choose **Options**, select the desired
graphics preference and save it, then restart PhotoCraft. Microsoft's
[graphics preference instructions](https://support.microsoft.com/en-us/windows/hardware/display-graphics/optimizations-for-windowed-games-in-windows-11)
describe this setting. PhotoCraft also ranks available adapters itself, so check
System Info after restarting instead of assuming the Windows preference was
applied.

Run `dxdiag` and inspect its Display/Render tabs to identify the GPU and installed
driver. [Microsoft Basic Display Adapter](https://support.microsoft.com/en-us/windows/hardware/display-graphics/microsoft-basic-display-adapter-in-windows)
is Windows' fallback display driver; install the applicable NVIDIA, Intel or PC
manufacturer driver to use the hardware graphics stack.

## PhotoCraft graphics recovery

Automatic selection prefers a discrete GPU when one can present to the window.
Intel adapters on Windows default to DirectX 12. If a driver crashes during
startup, the next launch tries the next backend in the **Vulkan → DirectX 12 →
CPU** sequence and remembers the successful choice. **Preferences → Performance
→ GPU Backend** and **Reset GPU Backend** let you revisit that choice after a
driver update.

For a recovery launch, open PowerShell in the directory containing the executable:

```powershell
.\photocraft.exe --safe-gpu
```

This uses the CPU canvas for that launch and selects a software window adapter
such as Windows WARP where available. Record the driver version and PhotoCraft's
System Info when reporting a graphics problem.
