# Crea WinFilesMenu.msix, il pacchetto che mette "Apri in WinFiles" nel menu corto di Esplora file
# di Windows 11 (ADR 0022), e lo firma con il certificato Winsport. Gira solo su Windows (CI).
#
#   installer/crea-pacchetto-menu.ps1 -Versione 0.2.14 -Dll target/release/winfiles_menu11.dll `
#       -Exe target/release/winfiles.exe -Uscita menu [-Prova]
#
# Il certificato arriva dai segreti WINSPORT_PFX (il file .pfx in base64) e WINSPORT_PFX_PASSWORD;
# senza segreti il pacchetto non si crea e l'MSI esce senza il menu di Windows 11.
# Con -Prova firma con un certificato usa e getta, registra il pacchetto su questo PC, chiede alla
# DLL il titolo della voce come farebbe Esplora file e poi toglie tutto: la prova della CI.
param(
    [Parameter(Mandatory)] [string] $Versione,
    [Parameter(Mandatory)] [string] $Dll,
    [Parameter(Mandatory)] [string] $Exe,
    [Parameter(Mandatory)] [string] $Uscita,
    [switch] $Prova
)
$ErrorActionPreference = 'Stop'

$lavoro = Join-Path ([IO.Path]::GetTempPath()) 'winfiles-pacchetto-menu'
Remove-Item -Recurse -Force $lavoro -ErrorAction SilentlyContinue
$contenuto = Join-Path $lavoro 'contenuto'
New-Item -ItemType Directory -Force $contenuto, $Uscita | Out-Null
$Uscita = (Resolve-Path $Uscita).Path

if (-not $Prova -and -not $env:WINSPORT_PFX) {
    Write-Host 'Nessun certificato Winsport nei segreti: il menu di Windows 11 resta fuori.'
    exit 0
}

# Il contenuto del pacchetto: solo il manifesto e i loghi; programma e DLL restano nella cartella
# dell'MSI ("posizione esterna").
Copy-Item -Recurse (Join-Path $PSScriptRoot 'menu-win11\Assets') $contenuto
$manifesto = Get-Content -Raw (Join-Path $PSScriptRoot 'menu-win11\AppxManifest.xml')
$manifesto = $manifesto.Replace('Version="0.0.0.0"', "Version=""$Versione.0""")
[IO.File]::WriteAllText((Join-Path $contenuto 'AppxManifest.xml'), $manifesto)

$kit = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64" -Directory |
    Where-Object { Test-Path (Join-Path $_.FullName 'makeappx.exe') } |
    Sort-Object Name -Descending | Select-Object -First 1
if (-not $kit) { throw 'Windows SDK non trovato (makeappx.exe)' }
$makeappx = Join-Path $kit.FullName 'makeappx.exe'
$signtool = Join-Path $kit.FullName 'signtool.exe'

$msix = Join-Path $Uscita 'WinFilesMenu.msix'
# /nv: il programma non è nel pacchetto, quindi niente controllo dei file dichiarati.
& $makeappx pack /d $contenuto /p $msix /nv /o
if ($LASTEXITCODE) { throw "makeappx non è riuscito ($LASTEXITCODE)" }

$pfx = Join-Path $lavoro 'certificato.pfx'
if ($Prova) {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Winsport' `
        -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddDays(1)
    $password = 'prova'
    Export-PfxCertificate -Cert $cert -FilePath $pfx `
        -Password (ConvertTo-SecureString $password -AsPlainText -Force) | Out-Null
} else {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINSPORT_PFX))
    $password = $env:WINSPORT_PFX_PASSWORD
}
& $signtool sign /fd SHA256 /f $pfx /p $password $msix
$firma = $LASTEXITCODE
Remove-Item -Force $pfx
if ($firma) { throw "signtool non è riuscito ($firma)" }
Copy-Item $Dll (Join-Path $Uscita 'winfiles_menu11.dll')
Write-Host "Creato $msix ($Versione)"

if (-not $Prova) { exit 0 }

# La prova: una cartella come quella dell'MSI, il certificato fra le Persone attendibili, la
# registrazione come la fa WinFiles, poi la DLL caricata da COM come la carica Esplora file.
$installato = Join-Path $lavoro 'installato'
New-Item -ItemType Directory -Force $installato | Out-Null
Copy-Item $Exe (Join-Path $installato 'WinFiles.exe')
Copy-Item $Dll (Join-Path $installato 'winfiles_menu11.dll')
$cer = Join-Path $lavoro 'prova.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
$fidato = Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
try {
    Add-AppxPackage -Path $msix -ExternalLocation $installato
    $pacchetto = Get-AppxPackage -Name 'Winsport.WinFiles'
    if (-not $pacchetto) { throw 'Il pacchetto non risulta registrato' }
    Write-Host "Registrato $($pacchetto.PackageFullName)"

    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

[ComImport, Guid("a08ce4d0-fa25-44ab-b57c-c7b1c323e0b9"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IExplorerCommand {
    [PreserveSig] int GetTitle(IntPtr items, [MarshalAs(UnmanagedType.LPWStr)] out string title);
}

public static class VoceMenu {
    public static string Titolo(string clsid) {
        object o = Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid(clsid), true));
        string titolo;
        int hr = ((IExplorerCommand)o).GetTitle(IntPtr.Zero, out titolo);
        Marshal.ReleaseComObject(o);
        if (hr != 0) { throw new Exception("GetTitle: 0x" + hr.ToString("X8")); }
        return titolo;
    }
}
'@
    $titolo = [VoceMenu]::Titolo('6E2C9B4A-3D71-4F85-A0C2-91B7E5D4F318')
    Write-Host "La DLL risponde: $titolo"
    if ($titolo -ne 'Apri in WinFiles') { throw "Titolo inatteso: $titolo" }
} finally {
    Get-AppxPackage -Name 'Winsport.WinFiles' | Remove-AppxPackage
    Remove-Item -Force "Cert:\LocalMachine\TrustedPeople\$($fidato.Thumbprint)"
    Remove-Item -Force "Cert:\CurrentUser\My\$($cert.Thumbprint)"
}
Write-Host 'Prova del menu di Windows 11 riuscita'
