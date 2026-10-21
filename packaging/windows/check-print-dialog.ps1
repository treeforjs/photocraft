<#
.SYNOPSIS
  Run the cross-built ARM64 print checks and capture the Windows PDF dialog offscreen.
.DESCRIPTION
  CI diagnostic only. Uses synthetic art, runs no physical printer, and installs nothing.
  The bundle and output must be separate folders beneath RUNNER_TEMP.
#>
param(
  [Parameter(Mandatory)] [string] $Bundle,
  [Parameter(Mandatory)] [string] $OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-TempChild([string] $Path) {
  if (-not $env:RUNNER_TEMP) { throw 'RUNNER_TEMP is required' }
  $parent = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
  $absolute = [IO.Path]::GetFullPath($Path)
  if (-not $absolute.StartsWith($parent, [StringComparison]::OrdinalIgnoreCase)) { throw 'Diagnostic path must be beneath RUNNER_TEMP' }
  return $absolute
}

function Assert-Arm64([string] $Path) {
  $stream = [IO.File]::OpenRead($Path)
  try {
    $dos = [byte[]]::new(64)
    $stream.ReadExactly($dos, 0, $dos.Length)
    $offset = [BitConverter]::ToInt32($dos, 60)
    if ($dos[0] -ne 0x4D -or $dos[1] -ne 0x5A -or $offset -lt 64 -or $offset -gt $stream.Length - 6) { throw 'Invalid executable header' }
    $stream.Position = $offset
    $pe = [byte[]]::new(6)
    $stream.ReadExactly($pe, 0, $pe.Length)
    if ([BitConverter]::ToUInt32($pe, 0) -ne 0x4550 -or [BitConverter]::ToUInt16($pe, 4) -ne 0xAA64) {
      throw "$Path is not an ARM64 Windows executable"
    }
  } finally { $stream.Dispose() }
}

function Assert-Png([string] $Path) {
  $stream = [IO.File]::OpenRead($Path)
  try {
    if ($stream.Length -le 1024 -or $stream.Length -gt 32MB) { throw 'Missing or implausibly sized dialog PNG' }
    $head = [byte[]]::new(24)
    $stream.ReadExactly($head, 0, $head.Length)
    if ([Convert]::ToHexString([byte[]]$head[0..7]) -ne '89504E470D0A1A0A' -or [Text.Encoding]::ASCII.GetString($head, 12, 4) -ne 'IHDR') {
      throw 'Invalid dialog PNG header'
    }
    if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($head, 16, 4); [Array]::Reverse($head, 20, 4) }
    if ([BitConverter]::ToUInt32($head, 16) -ne 1280 -or [BitConverter]::ToUInt32($head, 20) -ne 900) { throw 'Expected a 1280x900 dialog capture' }
  } finally { $stream.Dispose() }
}

function New-PrintScript([ValidateSet('light', 'dark')] [string] $Theme) {
  # ui.set is the same supported appearance control as the running desktop app.
  return ConvertTo-Json -Depth 12 -Compress -InputObject @(
    @('ui.set', @{ theme = $Theme }),
    @('engine.execute', @{ command = 'prefs.set'; params = @{ path = 'interface.language'; value = 'en' } }),
    @('engine.execute', @{ command = 'file.new'; params = @{ width = 64; height = 64; mode = 'rgb'; depth = 8; background = '#ffffff'; name = 'Print preview' } }),
    @('ui.menu.invoke', @{ id = 'file.print'; params = @{} })
  )
}

function Invoke-Check([string] $Executable, [string[]] $Arguments, [string] $Label, [int] $Seconds = 120) {
  $start = [Diagnostics.ProcessStartInfo]::new($Executable)
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.RedirectStandardOutput = $true
  $start.RedirectStandardError = $true
  foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
  # Headless kittest prefers software adapters; DX12 exposes the Windows ARM64 WARP adapter.
  $start.Environment['WGPU_BACKEND'] = 'dx12'
  $start.Environment['APPDATA'] = Join-Path $OutputDirectory 'profile\roaming'
  $start.Environment['LOCALAPPDATA'] = Join-Path $OutputDirectory 'profile\local'
  $process = [Diagnostics.Process]::new()
  $process.StartInfo = $start
  $timedOut = $false
  try {
    if (-not $process.Start()) { throw "Could not start $Label" }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($Seconds * 1000)) {
      $timedOut = $true
      $process.Kill($true)
      if (-not $process.WaitForExit(10000)) { throw "$Label did not stop after its timeout" }
    }
    $text = $stdout.GetAwaiter().GetResult()
    $errors = $stderr.GetAwaiter().GetResult()
    [IO.File]::WriteAllText((Join-Path $OutputDirectory "$Label.stdout.log"), $text)
    [IO.File]::WriteAllText((Join-Path $OutputDirectory "$Label.stderr.log"), $errors)
    if ($timedOut) { throw "$Label exceeded $Seconds seconds" }
    if ($process.ExitCode -ne 0) { throw "$Label failed with exit code $($process.ExitCode); see diagnostic logs" }
    if ($errors -match '(?m)^(ui\.set|engine\.execute|ui\.menu\.invoke): \{') { throw "$Label rejected a snapshot command; see diagnostic logs" }
    return $text
  } finally { $process.Dispose() }
}

$Bundle = Assert-TempChild $Bundle
$OutputDirectory = Assert-TempChild $OutputDirectory
if ($Bundle.TrimEnd('\', '/') -eq $OutputDirectory.TrimEnd('\', '/')) { throw 'Bundle and captures must use different folders' }
if (Test-Path -LiteralPath $OutputDirectory) { throw 'Capture directory must be fresh' }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
$source = (Get-Content -LiteralPath (Join-Path $Bundle 'source.txt') -Raw).Trim()
if ($source -notmatch '^[0-9a-f]{40}$' -or $source -ne $env:GITHUB_SHA) { throw 'Diagnostic binaries do not match this workflow commit' }
$executables = @{}
foreach ($name in 'engine-print-tests.exe', 'ui-print-tests.exe', 'snapshot.exe') {
  $path = Join-Path $Bundle $name
  Assert-Arm64 $path
  $executables[$name] = $path
}
$engine = Invoke-Check $executables['engine-print-tests.exe'] @('print_cmds::tests::', '--test-threads=1', '--color', 'never') 'engine-print' 180
if ($engine -notmatch 'test result: ok\.' -or $engine -notmatch 'windows_print_requires_pdf_output_and_refuses_physical_dispatch_before_replacing_files[^\r\n]*\.\.\. ok') {
  throw 'Windows engine print regression did not run successfully'
}
$ui = Invoke-Check $executables['ui-print-tests.exe'] @('file_ui::tests::windows_print_dialog_is_explicit_pdf_only_and_requires_a_destination', '--exact', '--test-threads=1', '--color', 'never') 'ui-print'
if ($ui -notmatch 'test result: ok\. 1 passed; 0 failed;') { throw 'Expected exactly one passing Windows print dialog test' }
$hashes = @{}
foreach ($theme in 'light', 'dark') {
  $path = Join-Path $OutputDirectory "print-dialog-$theme.png"
  $script = New-PrintScript $theme
  $null = Invoke-Check $executables['snapshot.exe'] @('--out', $path, '--safe-gpu', '--size', '1280x900', '--scale', '1', '--script', $script) "print-dialog-$theme"
  Assert-Png $path
  $hashes[$theme] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}
if ($hashes.light -eq $hashes.dark) { throw 'Light and dark captures must differ' }
@{ commit = $source; target = 'aarch64-pc-windows-msvc'; enginePrint = 'passed'; dialogPrint = 'passed'; captures = $hashes } |
  ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $OutputDirectory 'validation.json')
Write-Output 'Windows ARM64 PDF print tests and both dialog captures passed'
