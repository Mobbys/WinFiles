# Crea il certificato Winsport che firma il pacchetto del menu di Windows 11 (ADR 0022).
# Si esegue UNA volta sola, in PowerShell sul PC di chi gestisce WinFiles. Poi:
#  1. su GitHub, Mobbys/WinFiles > Settings > Secrets and variables > Actions > New repository
#     secret: WINSPORT_PFX = il testo copiato negli appunti (Ctrl+V);
#  2. un secondo segreto, WINSPORT_PFX_PASSWORD = la password scritta qui sotto;
#  3. conservare winsport.pfx e la password in un posto sicuro (chiavetta, cassaforte): con un
#     certificato nuovo ogni PC dovrebbe installarlo di nuovo.
# Il certificato vale 10 anni. Non va mai messo nel repository, che è pubblico.
$ErrorActionPreference = 'Stop'
$c = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Winsport' `
    -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(10) -KeyExportPolicy Exportable
$password = [guid]::NewGuid().ToString('N')
$pfx = Join-Path ([Environment]::GetFolderPath('Desktop')) 'winsport.pfx'
Export-PfxCertificate -Cert $c -FilePath $pfx -Password (ConvertTo-SecureString $password -AsPlainText -Force) | Out-Null
[Convert]::ToBase64String([IO.File]::ReadAllBytes($pfx)) | Set-Clipboard
Write-Host "Certificato salvato in $pfx e copiato negli appunti per WINSPORT_PFX."
Write-Host "Password per WINSPORT_PFX_PASSWORD: $password"
