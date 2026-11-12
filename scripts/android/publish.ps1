param(
    [Parameter(Mandatory=$true)][long]$RunId,
    [Parameter(Mandatory=$true)][string]$NotesFile,
    [string]$BuildTools="artifacts/android-build-tools/android-37.0"
)
$ErrorActionPreference="Stop"
Set-Location (Resolve-Path (Join-Path $PSScriptRoot "../.."))
$repo="yuuolo/prism"
$gh="C:/Program Files/GitHub CLI/gh.exe"
$java="C:/Program Files/Java/jdk-24/bin/java.exe"
function Assert-Exit {if($LASTEXITCODE -ne 0) {throw "External command failed ($LASTEXITCODE)"}}
$run=(& $gh run view $RunId --repo $repo --json status,conclusion,headSha | ConvertFrom-Json); Assert-Exit
if($run.status -ne "completed" -or $run.conclusion -ne "success") {throw "Build and emulator verification must pass before publishing"}
$work="artifacts/android-release-$RunId"
New-Item -ItemType Directory -Force $work | Out-Null
$unsigned=Join-Path $work "app-release-unsigned.apk"
if(-not (Test-Path $unsigned)) {& $gh run download $RunId --repo $repo --name prism-android-unsigned-release --dir $work; Assert-Exit}
$badging=(& (Join-Path $BuildTools "aapt2.exe") dump badging $unsigned) -join "`n"; Assert-Exit
if($badging -notmatch "package: name='dev.prism.android' versionCode='(\d+)' versionName='([0-9A-Za-z.+-]+)'") {throw "Unexpected Android package"}
$code=[long]$Matches[1]; $version=$Matches[2]
if($badging -match "application-debuggable") {throw "Debuggable APKs cannot be released"}
if($badging -notmatch "minSdkVersion:'(\d+)'") {throw "Missing minimum SDK"}
$minimum=[int]$Matches[1]
$tag="android-v$version"; $name="Prism-Android-$version.apk"; $apk=Join-Path $work $name
& $java --enable-native-access=ALL-UNNAMED -jar (Join-Path $BuildTools "lib/apksigner.jar") sign --ks android/.signing/prism-android.jks --ks-key-alias prism-android --ks-pass file:android/.signing/store-password.txt --out $apk $unsigned; Assert-Exit
$certificate=(& $java --enable-native-access=ALL-UNNAMED -jar (Join-Path $BuildTools "lib/apksigner.jar") verify --verbose --print-certs $apk) -join "`n"; Assert-Exit
if($certificate -notmatch 'certificate SHA-256 digest: 618f92202c7e120f365eebc928044ec68422bd12a26ba7337e43075336f6f1c0') {throw "Signing certificate differs from the published Android Alpha"}
& (Join-Path $BuildTools "zipalign.exe") -c -P 16 4 $apk; Assert-Exit
$hash=(Get-FileHash $apk -Algorithm SHA256).Hash.ToLowerInvariant()
$channel=& $gh api "repos/$repo/releases/tags/android-updates" 2>$null
if($LASTEXITCODE -eq 0 -and (($channel | ConvertFrom-Json).assets | Where-Object {$_.name -eq "android-update.json"})) {
    $previousDir=Join-Path $work ("channel-"+[Guid]::NewGuid().ToString("N"))
    & $gh release download android-updates --repo $repo --pattern android-update.json --dir $previousDir; Assert-Exit
    $previous=Get-Content (Join-Path $previousDir "android-update.json") -Raw | ConvertFrom-Json
    if([long]$previous.versionCode -gt $code -or ([long]$previous.versionCode -eq $code -and $previous.sha256 -ne $hash)) {throw "Refusing to roll back or replace an existing update version"}
}
$checksum="$apk.sha256"
[IO.File]::WriteAllText((Join-Path (Get-Location) $checksum),"$hash  $name`n",[Text.Encoding]::ASCII)
$existingText=& $gh api "repos/$repo/releases/tags/$tag" 2>$null
if($LASTEXITCODE -eq 0) {
    $existing=$existingText | ConvertFrom-Json
    $asset=$existing.assets | Where-Object {$_.name -eq $name}
    if(-not $asset -or $asset.digest -ne "sha256:$hash") {throw "Release already exists with different assets; never replace an APK for an existing version"}
} else {
    & $gh release create $tag $apk $checksum --repo $repo --target $run.headSha --title "Prism Android $version" --notes-file $NotesFile --prerelease --latest=false; Assert-Exit
}
# The only uploaded files are the signed APK, its hash, and public update metadata.
# Signing keys and passwords remain on this machine.
$metadata=[ordered]@{schemaVersion=1; versionCode=$code; versionName=$version; minSdk=$minimum; size=(Get-Item $apk).Length;
    sha256=$hash; apkUrl="https://github.com/$repo/releases/download/$tag/$name";
    notes=(Get-Content $NotesFile -Raw).Substring(0,[Math]::Min(16000,(Get-Content $NotesFile -Raw).Length)); publishedAt=[DateTime]::UtcNow.ToString("o")}
$manifest=Join-Path $work "android-update.json"
[IO.File]::WriteAllText((Join-Path (Get-Location) $manifest),($metadata | ConvertTo-Json -Depth 4),[Text.UTF8Encoding]::new($false))
& $gh release view android-updates --repo $repo --json tagName 2>$null | Out-Null
if($LASTEXITCODE -ne 0) {
    & $gh release create android-updates --repo $repo --target $run.headSha --title "Android 更新通道" --notes "仅提供安卓客户端的更新元数据，不改变桌面版更新入口。" --prerelease --latest=false; Assert-Exit
}
& $gh release upload android-updates $manifest --repo $repo --clobber; Assert-Exit
& $gh api "repos/$repo/releases/tags/$tag" --jq '.assets[]|{name,digest}'; Assert-Exit
Write-Output "https://github.com/$repo/releases/tag/$tag"
