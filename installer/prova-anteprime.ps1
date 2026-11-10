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
    $chiave = 'HKCU:\' + $v.GetAttribute('Key')
    if (-not (Test-Path $chiave)) { New-Item -Path $chiave -Force | Out-Null }
    $valore = $v.GetAttribute('Value').Replace('[INSTALLDIR]', $dir)
    $tipo = if ($v.GetAttribute('Type') -eq 'integer') { 'DWord' } else { 'String' }
    $nome = if ($v.GetAttribute("Name")) { $v.GetAttribute("Name") } else { '(default)' }
    New-ItemProperty -Path $chiave -Name $nome -Value $valore -PropertyType $tipo -Force | Out-Null
}
# New-Item -Force su una chiave che c'è già ne cancella i valori: si crea solo se manca.
Write-Host "Registrate $($valori.Count) chiavi; DLL in $dir"
# Sul PC della CI tutto gira come amministratore, e un processo elevato non guarda le classi COM
# registrate per l'utente (HKCU\Software\Classes\CLSID): la classe va anche nella macchina,
# altrimenti Windows risponde "classe non registrata" senza mai caricare la DLL.
foreach ($v in $valori | Where-Object { $_.GetAttribute('Key') -like 'Software\Classes\CLSID\*' }) {
    $chiave = 'HKLM:\' + $v.GetAttribute('Key')
    if (-not (Test-Path $chiave)) { New-Item -Path $chiave -Force | Out-Null }
    $nome = if ($v.GetAttribute("Name")) { $v.GetAttribute("Name") } else { '(default)' }
    New-ItemProperty -Path $chiave -Name $nome -Value $v.GetAttribute('Value').Replace('[INSTALLDIR]', $dir) -PropertyType String -Force | Out-Null
}
foreach ($k in 'HKCU\Software\Classes\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}', 'HKLM\Software\Classes\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}', 'HKCR\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}') {
    Write-Host "reg query $k"; reg query $k /s
}
# Un programma che si è preso il PSD con il suo gestore delle anteprime, come fa Photoshop:
# DllRegisterServer deve mettere davanti quello di WinFiles.
New-Item -Path 'HKCU:\Software\Classes\.psd' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\.psd' -Name '(default)' -Value 'Prova.Immagine'
New-Item -Path 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}' -Force | Out-Null
Set-ItemProperty -Path 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}' -Name '(default)' -Value '{00000000-0000-0000-0000-000000000001}'
# Come sul PC di Michel: la classe per l'utente non c'è (l'MSI a 32 bit la mette in WOW6432Node)
# e la deve scrivere DllRegisterServer.
Remove-Item -Path 'HKCU:\Software\Classes\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}' -Recurse -Force -ErrorAction SilentlyContinue
$p = Start-Process regsvr32 -ArgumentList '/s', (Join-Path $dir 'winfiles_preview.dll') -Wait -PassThru
if ($p.ExitCode -ne 0) {
    Get-Content (Join-Path $env:TEMP 'winfiles-anteprime\errori.txt') -ErrorAction SilentlyContinue
    throw "DllRegisterServer: $($p.ExitCode)"
}
$srv = (Get-ItemProperty 'HKCU:\Software\Classes\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}\InprocServer32' -ErrorAction SilentlyContinue).'(default)'
if ($srv -ne (Join-Path $dir 'winfiles_preview.dll')) { throw "DllRegisterServer non ha scritto la classe: '$srv'" }
Write-Host "DllRegisterServer: classe scritta ($srv)"
$h = (Get-ItemProperty 'HKCU:\Software\Classes\Prova.Immagine\ShellEx\{e357fccd-a995-4576-b01f-234630154e96}').'(default)'
if ($h -ne '{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}') { throw "Il gestore nel ProgID non è quello di WinFiles: $h" }
Write-Host "DllRegisterServer: gestore di WinFiles anche nel ProgID Prova.Immagine"
# Il file "traccia" fa scrivere alla DLL ogni passo nel suo registro.
foreach ($c in @((Join-Path $env:TEMP 'winfiles-anteprime'), (Join-Path (Split-Path $env:LOCALAPPDATA) 'LocalLow\WinFiles\anteprime'), (Join-Path $env:TEMP 'Low\winfiles-anteprime'))) {
    New-Item -ItemType Directory -Force -Path $c | Out-Null
    New-Item -ItemType File -Force -Path (Join-Path $c 'traccia') | Out-Null
}
& (Join-Path $dir 'examples\esplora.exe') (Join-Path $env:RUNNER_TEMP 'anteprime')
$esito = $LASTEXITCODE
# Per capire: le stesse anteprime con la DLL caricata nel processo che le chiede, senza il processo
# a parte di Windows (DisableProcessIsolation), su file nuovi.
foreach ($radice in 'HKCU:\', 'HKLM:\') {
    Set-ItemProperty -Path ($radice + 'Software\Classes\CLSID\{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}') -Name 'DisableProcessIsolation' -Value 1 -Type DWord
}
Write-Host "--- senza processo a parte (solo per capire):"
& (Join-Path $dir 'examples\esplora.exe') (Join-Path $env:RUNNER_TEMP 'anteprime-nel-processo')
$cartelle = @(
    (Join-Path $env:TEMP 'winfiles-anteprime'),
    (Join-Path (Split-Path $env:LOCALAPPDATA) 'LocalLow\WinFiles\anteprime'),
    (Join-Path $env:TEMP 'Low\winfiles-anteprime')
)
foreach ($c in $cartelle) {
    $errori = Join-Path $c 'errori.txt'
    if (Test-Path $errori) { Write-Host "Errori della DLL ($errori):"; Get-Content $errori }
}
Get-Process dllhost -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "dllhost $($_.Id): $((Get-Process -Id $_.Id -Module -ErrorAction SilentlyContinue | Where-Object ModuleName -like 'winfiles*').FileName)" }
# La diagnosi che si lancia sui PC degli utenti con rundll32.
# (Start-Process -Wait aspetterebbe anche il Blocco note che la diagnosi apre: si aspetta solo rundll32.)
$p = Start-Process rundll32 -ArgumentList ('"' + (Join-Path $dir 'winfiles_preview.dll') + '",Diagnosi') -PassThru
if (-not $p.WaitForExit(300000)) { throw "La diagnosi non ha finito in 5 minuti" }
$diagnosi = Join-Path $env:TEMP 'winfiles-diagnosi.txt'
if (-not (Test-Path $diagnosi)) { throw "La diagnosi non ha scritto il resoconto" }
Get-Content $diagnosi
Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -ErrorAction SilentlyContinue
if ($esito -ne 0) { throw "Anteprime mancanti" }
