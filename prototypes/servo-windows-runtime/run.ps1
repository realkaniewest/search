# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.
$ErrorActionPreference = 'Stop'

$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$work = Join-Path $env:RUNNER_TEMP ('search-servo-windows-' + [guid]::NewGuid().ToString('N'))
$archive = Join-Path $work 'servo-x86_64-windows-msvc.zip'
$png = Join-Path $work 'screen.png'
$serverLog = Join-Path $work 'server.log'
$serverErr = Join-Path $work 'server.err'
$servoLog = Join-Path $work 'servo.log'
$servoErr = Join-Path $work 'servo.err'
$profile = Join-Path $work 'profile'
$url = 'http://127.0.0.1:43132/probe.html'
$expected = '0fca2f4d7e6107304631f7b9031edf0df2c3d88e0570cd19cde1'
$server = $null
$servo = $null
New-Item -ItemType Directory -Path $work | Out-Null

try {
  Invoke-WebRequest -Uri 'https://github.com/servo/servo/releases/download/v0.8.0/servo-x86_64-windows-msvc.zip' -OutFile $archive
  $actual = (Get-FileHash -Path $archive -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected) { throw "Servo archive SHA-256 mismatch: $actual" }
  Write-Output 'SHA256: official Servo ZIP matches pinned digest'

  Expand-Archive -Path $archive -DestinationPath (Join-Path $work 'unpacked')
  $exe = Get-ChildItem (Join-Path $work 'unpacked') -Recurse -Filter servoshell.exe -File | Select-Object -First 1 -ExpandProperty FullName
  if (-not $exe) { throw 'servoshell.exe missing from official ZIP' }

  $fixture = Join-Path $root 'prototypes/servo-windows-runtime'
  $server = Start-Process -FilePath 'python' -ArgumentList @('-u', '-m', 'http.server', '43132', '--bind', '127.0.0.1', '--directory', $fixture) -PassThru -RedirectStandardOutput $serverLog -RedirectStandardError $serverErr
  $ready = $false
  for ($i = 0; $i -lt 40; $i++) {
    if ($server.HasExited) { throw "Local server exited early: $(Get-Content $serverErr -Raw)" }
    try {
      $response = Invoke-WebRequest -Method Head -Uri $url -TimeoutSec 2
      if ($response.StatusCode -eq 200) { $ready = $true; break }
    } catch { Start-Sleep -Milliseconds 250 }
  }
  if (-not $ready) { throw 'Local server did not become ready' }

  $servo = Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe) -ArgumentList @('--headless', '--exit', "--output=$png", "--config-dir=$profile", $url) -PassThru -RedirectStandardOutput $servoLog -RedirectStandardError $servoErr
  if (-not $servo.WaitForExit(90000)) { throw 'ServoShell exceeded 90 seconds' }
  $servo.Refresh()
  if ($servo.ExitCode -ne 0) {
    if (Test-Path $servoErr) { Get-Content $servoErr | Write-Output }
    throw "ServoShell exited with code $($servo.ExitCode)"
  }
  if (-not (Test-Path $png)) { throw 'ServoShell exited without PNG output' }
  if (-not (Select-String -Path $serverErr -Pattern 'GET /probe.html HTTP/1.1" 200' -Quiet)) { throw 'No GET from ServoShell in local server log' }

  Add-Type -AssemblyName System.Drawing
  $bitmap = [System.Drawing.Bitmap]::new($png)
  try {
    $pixels = 0
    for ($y = 0; $y -lt $bitmap.Height; $y += 4) {
      for ($x = 0; $x -lt $bitmap.Width; $x += 4) {
        $color = $bitmap.GetPixel($x, $y)
        if ($color.R -eq 20 -and $color.G -eq 184 -and $color.B -eq 166) { $pixels++ }
      }
    }
    if ($pixels -le 1000) { throw "Expected JavaScript-colored pixels missing: $pixels" }
    Write-Output "PASS: ServoShell requested local page and rendered $pixels sampled JavaScript-colored pixels in $($bitmap.Width)x$($bitmap.Height) PNG"
  } finally { $bitmap.Dispose() }
} finally {
  foreach ($process in @($servo, $server)) {
    if ($null -ne $process -and -not $process.HasExited) {
      & taskkill.exe /PID $process.Id /T /F 2>$null | Out-Null
    }
  }
  Remove-Item -Path $work -Recurse -Force -ErrorAction SilentlyContinue
}
