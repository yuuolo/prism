# Disposable elevated Windows runner only. Installs a real TUN for this test.
param([Parameter(Mandatory=$true)][string]$Core)
$ErrorActionPreference = 'Stop'
$testDir = Join-Path $env:RUNNER_TEMP 'prism-dns-test'
New-Item -ItemType Directory -Path $testDir -Force | Out-Null
$archive = Join-Path $testDir 'wintun.zip'
Invoke-WebRequest https://www.wintun.net/builds/wintun-0.14.1.zip -OutFile $archive
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '07c256185d6ee3652e09fa55c0b673e2624b565e02c4b9091c79ca7d2f24ef51') { throw 'wintun checksum mismatch' }
Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $testDir 'driver') -Force
Copy-Item -LiteralPath (Join-Path $testDir 'driver/wintun/bin/amd64/wintun.dll') -Destination $testDir
$config = Join-Path $testDir 'config.yaml'
@'
mixed-port: 17891
external-controller: 127.0.0.1:19091
secret: ''
log-level: info
tun:
  enable: true
  auto-route: true
  inet6-address: ['fdfe:dcba:9876::1/126']
dns:
  enable: true
  ipv6: true
  nameserver: ['https://1.1.1.1/dns-query']
hosts:
  dns-guard-test.invalid: 203.0.113.7
proxies: []
rules: ['MATCH,DIRECT']
'@ | Set-Content -LiteralPath $config -Encoding utf8
$stdout = Join-Path $testDir 'core.log'
$stderr = Join-Path $testDir 'core-error.log'
$process = $null
try {
  $process = Start-Process -FilePath $Core -ArgumentList @('-f', $config, '-d', $testDir) -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
  $ready = $false
  for ($i = 0; $i -lt 45; $i++) {
    if ($process.HasExited) { throw "core exited: $($process.ExitCode)" }
    try {
      $log = Get-Content -LiteralPath $stdout -Raw
      if ($log -match 'TUN adapter Prism .* up at') { $ready = $true; break }
    } catch {}
    Start-Sleep -Seconds 1
  }
  if (!$ready) { throw 'TUN did not become ready' }
  if ((Get-Content -LiteralPath $stdout -Raw) -notmatch 'DNS protection: Windows IPv4/IPv6 TCP/UDP 53') { throw 'DNS enforcement was not installed' }
  foreach ($tcp in @($false, $true)) {
    Clear-DnsClientCache
    $dnsArgs = @{ Name='dns-guard-test.invalid'; Type='A'; Server='172.19.0.2'; DnsOnly=$true; NoHostsFile=$true; QuickTimeout=$true; ErrorAction='Stop' }
    if ($tcp) { $dnsArgs.TcpOnly = $true }
    $answer = Resolve-DnsName @dnsArgs
    if (@($answer | Where-Object IPAddress -eq '203.0.113.7').Count -eq 0) { throw "wrong TUN DNS answer, TCP=$tcp" }
    Write-Output "PASS actual TUN DNS response, TCP=$tcp"
  }
  # A hot reload must not remove interface protection or DNS interception.
  Invoke-RestMethod -Uri http://127.0.0.1:19091/configs -Method Put -ContentType application/json -Body (@{path=$config} | ConvertTo-Json) | Out-Null
  Clear-DnsClientCache
  $answer = Resolve-DnsName dns-guard-test.invalid -Type A -Server 172.19.0.2 -DnsOnly -NoHostsFile -QuickTimeout
  if (@($answer | Where-Object IPAddress -eq '203.0.113.7').Count -eq 0) { throw 'DNS failed after reload' }
  Write-Output 'PASS DNS interception survives reload'
} finally {
  if ($process -and !$process.HasExited) { Stop-Process -Id $process.Id -Force; $process.WaitForExit() }
  if (Test-Path -LiteralPath $stdout) { Get-Content -LiteralPath $stdout }
  if (Test-Path -LiteralPath $stderr) { Get-Content -LiteralPath $stderr }
}
