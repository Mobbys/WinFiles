# Prova dell'MSI vero su Windows (CI): l'aggiornamento da un rilascio a 32 bit, la classe delle
# anteprime nella vista a 64 bit del registro (quella che legge Esplora file) e la disinstallazione.
param([Parameter(Mandatory = $true)][string]$Msi)
$ErrorActionPreference = 'Stop'
$clsid = '{8B3C7E21-5D4A-4F96-9C1B-2E7A6D0F4B53}'
$key = "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32"
$dir = Join-Path $env:LOCALAPPDATA 'Programs\WinFiles'

function Invoke-Msi([string[]]$Arguments, [string]$What) {
    $log = Join-Path $env:RUNNER_TEMP "$What.log"
    $p = Start-Process msiexec -ArgumentList ($Arguments + @('/qn', '/norestart', '/l*v', "`"$log`"")) -PassThru
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) {
        Get-Content $log -Tail 60
        throw "${What}: msiexec ha risposto $($p.ExitCode)"
    }
    Write-Host "${What}: fatto"
}

if (-not [Environment]::Is64BitProcess) { throw 'La prova deve girare a 64 bit' }

# Un rilascio vecchio (pacchetto a 32 bit), per provare l'aggiornamento come sui PC degli utenti.
$old = Join-Path $env:RUNNER_TEMP 'WinFiles-vecchio.msi'
$oldOk = $true
try {
    Invoke-WebRequest 'https://github.com/Mobbys/WinFiles/releases/download/v0.2.33/WinFiles-0.2.33.msi' -OutFile $old
} catch {
    Write-Host "Rilascio 0.2.33 non scaricato ($_): si prova solo l'installazione nuova"
    $oldOk = $false
}
if ($oldOk) { Invoke-Msi @('/i', "`"$old`"") 'installazione-0.2.33' }

Invoke-Msi @('/i', "`"$((Resolve-Path $Msi).Path)`"") 'installazione'
$dll = Join-Path $dir 'winfiles_preview.dll'
foreach ($f in @($dll, (Join-Path $dir 'winfiles.exe'))) {
    if (-not (Test-Path $f)) { throw "Manca $f dopo l'installazione" }
}
$srv = (Get-ItemProperty $key -ErrorAction SilentlyContinue).'(default)'
if ($srv -ne $dll) { throw "La classe delle anteprime non è nella vista a 64 bit: '$srv' invece di '$dll'" }
$model = (Get-ItemProperty $key).ThreadingModel
if ($model -ne 'Apartment') { throw "ThreadingModel: '$model'" }
Write-Host "Classe delle anteprime dopo l'installazione: $srv"
$n = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue |
    Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq 'WinFiles' }).Count
if ($n -gt 1) { throw "WinFiles installato $n volte dopo l'aggiornamento" }

Invoke-Msi @('/x', "`"$((Resolve-Path $Msi).Path)`"") 'disinstallazione'
if (Test-Path $dll) { throw "La DLL è rimasta dopo la disinstallazione" }
if (Test-Path "HKCU:\Software\Classes\CLSID\$clsid") { throw 'La classe è rimasta dopo la disinstallazione' }
Write-Host 'Installer: aggiornamento, classe a 64 bit e disinstallazione a posto'
