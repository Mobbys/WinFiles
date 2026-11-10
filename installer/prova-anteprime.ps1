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
# Un programma che si è preso il PSD con il suo gestore delle anteprime, come fa Photoshop:
# DllRegisterServer deve mettere davanti quello di WinFiles.
New-Item -Path 'HKCU:\Software\Classes\.psd' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\.psd' -Name '(default)' -Value 'Prova.Immagine'
New-Item -Path 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}' -Name '(default)' -Value '{00000000-0000-0000-0000-000000000001}'
$p = Start-Process regsvr32 -ArgumentList '/s', (Join-Path $dir 'winfiles_preview.dll') -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "DllRegisterServer: $($p.ExitCode)" }
$h = (Get-ItemProperty 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}').'(default)'
if ($h -ne '{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}') { throw "Il gestore nel ProgID non è quello di WinFiles: $h" }
Write-Host "DllRegisterServer: gestore di WinFiles anche nel ProgID Prova.Immagine"
& (Join-Path $dir 'examples\esplora.exe') (Join-Path $env:RUNNER_TEMP 'anteprime')
$esito = $LASTEXITCODE
$errori = Join-Path $env:TEMP 'winfiles-anteprime\errori.txt'
if (Test-Path $errori) { Write-Host "Errori della DLL:"; Get-Content $errori }
if ($esito -ne 0) { throw "Anteprime mancanti" }
