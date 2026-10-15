//! Su Windows mette dentro `winfiles.exe` l'icona e i dati che Esplora file mostra nelle
//! proprietà (nome, versione, produttore). Sugli altri sistemi non fa nulla.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon/icona.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon/icona.ico")
        .set("ProductName", "WinFiles")
        .set(
            "FileDescription",
            "WinFiles: stampa delle miniature di file e cartelle",
        )
        .set("CompanyName", "Winsport")
        .set("LegalCopyright", "© 2026 Mobbys. Tutti i diritti riservati.")
        .set_language(0x0410);
    // Senza le risorse l'eseguibile funziona lo stesso: si avvisa e si va avanti.
    if let Err(e) = res.compile() {
        println!("cargo:warning=icona dell'eseguibile non inserita: {e}");
    }
}
