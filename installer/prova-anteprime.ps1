# Prova delle anteprime in Esplora file (ADR 0036) sul PC della CI: scrive in HKCU le stesse
# chiavi del componente "Anteprime" di winfiles.wxs, con la DLL della cartella data, e chiede a
# Windows le anteprime di alcuni file come fa Esplora file.
# Uso: installer/prova-anteprime.ps1 -Cartella target/release
param([Parameter(Mandatory = $true)][string]$Cartella)
$ErrorActionPreference = 'Stop'
$dir = (Resolve-Path $Cartella).Path.TrimEnd('\') + '\'
[xml]$wxs = Get-Content (Join-Path $PSScriptRoot 'winfiles.wxs') -Raw
$ns = New-Object Xml.XmlNamespaceManager $wxs.NameTable
$ns.AddNamespace('w', 'http://schemas.microsoft.com/wix/2006/wi')
$valori = $wxs.SelectNodes("//w:Component[@Id='Anteprime']/w:RegistryValue", $ns)
if ($valori.Count -lt 3) { throw "Componente Anteprime non trovato in winfiles.wxs" }
foreach ($v in $valori) {
    $chiave = 'HKCU:\' + $v.Key
    New-Item -Path $chiave -Force | Out-Null
    $valore = $v.Value.Replace('[INSTALLDIR]', $dir)
    $tipo = if ($v.Type -eq 'integer') { 'DWord' } else { 'String' }
    $nome = if ($v.Name) { $v.Name } else { '(default)' }
    New-ItemProperty -Path $chiave -Name $nome -Value $valore -PropertyType $tipo -Force | Out-Null
}
Write-Host "Registrate $($valori.Count) chiavi; DLL in $dir"
& (Join-Path $dir 'examples\esplora.exe') (Join-Path $env:RUNNER_TEMP 'anteprime')
if ($LASTEXITCODE -ne 0) { throw "Anteprime mancanti" }
